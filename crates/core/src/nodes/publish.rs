//! Publishing: the op log is the outbox. After every commit in a followed
//! space, and once it is followed, this device publishes its own applied ops
//! that `carbon_published` does not yet hold, in the shared order, each with
//! the effects its first run recorded; once p2panda accepts one, the core
//! records it. A crash between the two publishes it once more, and peers drop
//! the duplicate by op hash.
//!
//! What the core writes to a space's file for its p2panda space waits until
//! no op applies there, since an op's transaction holds the file.

use std::rc::Rc;
use std::sync::Arc;

use super::on_network;
use super::spaces::{Followed, Unrecorded};
use crate::carbon::Carbon;
use crate::channels::service::ServiceSpace;
use crate::error::{Result, internal};
use crate::ops::{errors, published};

impl Carbon {
  // Publish this device's applied ops in a space that are not yet published.
  pub(crate) fn publish(self: &Rc<Self>, space: &str) {
    let (Some(followed), Some(opened)) = (self.followed(space), self.opened(space)) else { return };
    if opened.is_applying() {
      return;
    }
    if let Err(failure) = self.publishable(&opened, &followed).map(|rows| rows.into_iter().for_each(|row| self.publish_row(&followed, space, row))) {
      self.stop(&opened, failure);
    }
  }

  // Write what waits for the space's file, then read the ops to publish that are not being published now.
  fn publishable(&self, opened: &ServiceSpace, followed: &Followed) -> Result<Vec<crate::ops::oplog::LogRow>> {
    record_unrecorded(opened, followed)?;
    let rows = published::unpublished(opened.store(), &followed.device)?;
    Ok(rows.into_iter().filter(|row| !followed.publishing.borrow().contains(&row.op.hash)).collect())
  }

  fn publish_row(self: &Rc<Self>, followed: &Rc<Followed>, space: &str, row: crate::ops::oplog::LogRow) {
    let hash = row.op.hash.clone();
    followed.publishing.borrow_mut().insert(hash.clone());
    let target = Arc::clone(&followed.space);
    let publishing = async move {
      let accepted = target.publish(row).await.map_err(|error| internal(format!("Cannot publish an op: {error}")))?;
      accepted.await.map_err(|error| internal(format!("p2panda did not take an op: {error}")))
    };
    let (carbon, kept, space) = (Rc::clone(self), Rc::clone(followed), space.to_string());
    on_network(self, publishing, move |accepted| {
      kept.publishing.borrow_mut().remove(&hash);
      match accepted {
        Ok(()) => {
          kept.unrecorded.borrow_mut().push(Unrecorded::Published(hash));
          carbon.record_between_ops(&space);
        }
        Err(error) => eprintln!("[carbondb] {space} did not publish {hash}: {}", error.message),
      }
    });
  }

  // Write what waits for a space's file now, unless an op applies there; the next commit writes it then.
  pub(crate) fn record_between_ops(self: &Rc<Self>, space: &str) {
    let (Some(followed), Some(opened)) = (self.followed(space), self.opened(space)) else { return };
    if opened.is_applying() {
      return;
    }
    if let Err(failure) = record_unrecorded(&opened, &followed) {
      self.stop(&opened, failure);
    }
  }
}

fn record_unrecorded(opened: &ServiceSpace, followed: &Followed) -> Result<()> {
  let waiting = std::mem::take(&mut *followed.unrecorded.borrow_mut());
  waiting.iter().try_for_each(|write| match write {
    Unrecorded::Published(hash) => published::record(opened.store(), hash),
    Unrecorded::Error(evidence) => errors::record(opened.store(), evidence),
  })
}
