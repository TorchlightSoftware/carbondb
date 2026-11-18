//! Publishing: a channel's log is its outbox. After every commit in a
//! followed channel, and once it is followed, this device publishes its own
//! ops that are publishable and not yet published, in the shared order, each
//! with the effects its first run recorded and its cause: those that applied
//! here, and whose cause did too, all the way up (`crate::ops::published`).
//! Once p2panda accepts one, the core records it in the log. A crash between
//! the two publishes it once more, and peers drop the duplicate by op hash.

use std::rc::Rc;
use std::sync::Arc;

use super::on_network;
use super::spaces::Followed;
use crate::carbon::Carbon;
use crate::error::{Result, internal};
use crate::ops::oplog::LogRow;
use crate::ops::published;

impl Carbon {
  // Publish this device's publishable ops in a channel that are not yet published.
  pub(crate) fn publish(self: &Rc<Self>, channel: &str) {
    let Some(followed) = self.followed(channel) else { return };
    match self.publishable(channel, &followed) {
      Ok(rows) => rows.into_iter().for_each(|row| self.publish_row(&followed, channel, row)),
      Err(error) => eprintln!("[carbondb] {channel} did not publish: {}", error.message),
    }
  }

  // The ops to publish that are not being published now.
  fn publishable(&self, channel: &str, followed: &Followed) -> Result<Vec<LogRow>> {
    let rows = published::unpublished(self.channel(channel)?.log(), &followed.device)?;
    Ok(rows.into_iter().filter(|row| !followed.publishing.borrow().contains(&row.op.hash)).collect())
  }

  fn publish_row(self: &Rc<Self>, followed: &Rc<Followed>, channel: &str, row: LogRow) {
    let hash = row.op.hash.clone();
    followed.publishing.borrow_mut().insert(hash.clone());
    let target = Arc::clone(&followed.space);
    let publishing = async move {
      let accepted = target.publish(row).await.map_err(|error| internal(format!("Cannot publish an op: {error}")))?;
      accepted.await.map_err(|error| internal(format!("p2panda did not take an op: {error}")))
    };
    let (carbon, kept, channel) = (Rc::clone(self), Rc::clone(followed), channel.to_string());
    on_network(self, publishing, move |accepted| {
      kept.publishing.borrow_mut().remove(&hash);
      let recorded = accepted.and_then(|()| carbon.channel(&channel)).and_then(|opened| published::record(opened.log(), &hash));
      if let Err(error) = recorded {
        eprintln!("[carbondb] {channel} did not publish {hash}: {}", error.message);
      }
    });
  }
}
