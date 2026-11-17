//! Receiving: every message that arrives on a followed p2panda space passes
//! one check before the op log, which decides whether it is an op at all; once
//! it is, it takes the same path as every op (`Carbon::receive`), and is
//! acknowledged once its row is logged.
//!
//! The check recomputes the op's hash from its envelope, checks that its
//! device is its p2panda author, and that its device belongs to its author
//! identity: the identity's first device, whose key the identity's id derives
//! from, or a device the identity's `[identity]` lists on this device. The
//! p2panda signature covers the effects, which the op's hash does not.
//!
//! - A message that fails is acknowledged and kept as evidence in
//!   `carbon_error`, never applied.
//! - One whose device is not yet listed is held unacknowledged and offered
//!   again after every commit, since the op listing a newly enrolled device
//!   can arrive later, from another space; after `UNLISTED_FOR` it fails as an
//!   unlisted device.
//! - One refused for now (425, too far ahead of this device's clock) is held
//!   and offered again once the clock has caught up.
//!
//! An author's later messages wait behind one of theirs held, since an
//! acknowledgement is a high-water mark per author log. A device's own
//! published op that p2panda echoes back is already logged, and is
//! acknowledged at once.

use std::rc::Rc;
use std::time::Duration;

use carbondb_svc_identity::id_of;
use serde_json::json;

use super::spaces::{Arrival, Followed, Held, Unrecorded};
use crate::carbon::Carbon;
use crate::Op;
use crate::ops::clock;
use crate::ops::errors::Evidence;
use crate::ops::op::hash_holds;
use crate::services::carbon as local;

// How long a message whose device no `[identity]` lists is held before it fails.
pub const UNLISTED_FOR: Duration = Duration::from_secs(2 * 60 * 60);

// What the check makes of a message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Check {
  Holds,
  Fails(&'static str),
  Unlisted,
}

// Check a message's op against its p2panda author, and whether its device belongs to its author identity.
pub(crate) fn check(author: &str, op: &Op, listed: impl Fn(&str, &str) -> bool) -> Check {
  let (Some(identity), Some(device)) = (op.author.as_deref(), op.device.as_deref()) else { return Check::Fails("an op from no identity or device") };
  if !hash_holds(op) {
    return Check::Fails("hash");
  }
  if device != author {
    return Check::Fails("device is not the p2panda author");
  }
  match id_of(device) == identity || listed(identity, device) {
    true => Check::Holds,
    false => Check::Unlisted,
  }
}

impl Carbon {
  // A message arrived on a followed space.
  pub(crate) fn arrived(self: &Rc<Self>, arrival: Arrival) {
    let Some(followed) = self.followed(&arrival.space) else { return };
    if arrival.own {
      return followed.ack(arrival.operation);
    }
    if followed.held.borrow().iter().any(|held| held.arrival.author == arrival.author) {
      return followed.held.borrow_mut().push(Held { arrival, expiring: false });
    }
    self.offer(&followed, Held { arrival, expiring: false });
  }

  // Offer a message to the check and the op log, holding it if it cannot enter yet.
  fn offer(self: &Rc<Self>, followed: &Rc<Followed>, held: Held) {
    match check(&held.arrival.author, &held.arrival.row.op, |identity, device| self.lists(identity, device)) {
      Check::Holds => self.log_message(followed, held),
      Check::Fails(failed) => self.refuse_message(followed, &held.arrival, failed),
      Check::Unlisted => {
        if !held.expiring {
          self.expire_later(&held.arrival.space, held.arrival.operation);
        }
        followed.held.borrow_mut().push(Held { expiring: true, ..held });
      }
    }
  }

  // Log a message's op, and acknowledge it once logged; hold it when it is too early.
  fn log_message(self: &Rc<Self>, followed: &Rc<Followed>, held: Held) {
    let (carbon, kept, time) = (Rc::clone(self), Rc::clone(followed), held.arrival.row.op.time);
    let (space, operation) = (held.arrival.space.clone(), held.arrival.operation);
    let row = held.arrival.row.clone();
    self.receive(&space.clone(), row, Box::new(move |received| match received {
      Ok(()) => kept.ack(operation),
      Err(error) if error.status == 425 => {
        kept.held.borrow_mut().push(held);
        carbon.offer_later(&space, Duration::from_millis(time.saturating_sub(clock::wall().saturating_add(clock::AHEAD)) + 1));
      }
      Err(error) => eprintln!("[carbondb] {space} could not take a message: {}", error.message),
    }));
  }

  // Keep a message that failed the check as evidence, and acknowledge it.
  fn refuse_message(self: &Rc<Self>, followed: &Followed, arrival: &Arrival, failed: &str) {
    let evidence = Evidence { operation: arrival.operation.to_hex(), author: arrival.author.clone(), arrived: clock::wall(), failed: failed.to_string(), signed: arrival.signed.clone() };
    followed.unrecorded.borrow_mut().push(Unrecorded::Error(evidence));
    followed.ack(arrival.operation);
    self.record_between_ops(&arrival.space);
  }

  // Offer a space's held messages again, in the order they arrived.
  pub(crate) fn offer_held(self: &Rc<Self>, space: &str) {
    let Some(followed) = self.followed(space) else { return };
    let held = std::mem::take(&mut *followed.held.borrow_mut());
    for one in held {
      if followed.held.borrow().iter().any(|waiting| waiting.arrival.author == one.arrival.author) {
        followed.held.borrow_mut().push(one);
      } else {
        self.offer(&followed, one);
      }
    }
  }

  // A message whose device is still unlisted `UNLISTED_FOR` after it was first held fails the check.
  fn expire_later(self: &Rc<Self>, space: &str, operation: p2panda::Hash) {
    let (carbon, space) = (Rc::downgrade(self), space.to_string());
    let _ = self.after(UNLISTED_FOR, Box::new(move || carbon.upgrade().into_iter().for_each(|carbon| carbon.expire(&space, operation))));
  }

  fn expire(self: &Rc<Self>, space: &str, operation: p2panda::Hash) {
    let Some(followed) = self.followed(space) else { return };
    let position = followed.held.borrow().iter().position(|held| held.arrival.operation == operation);
    let Some(held) = position.map(|position| followed.held.borrow_mut().remove(position)) else { return };
    match check(&held.arrival.author, &held.arrival.row.op, |identity, device| self.lists(identity, device)) {
      Check::Unlisted => self.refuse_message(&followed, &held.arrival, "unlisted device"),
      _ => self.offer(&followed, held),
    }
    self.offer_held(space);
  }

  // Offer a space's held messages again after a wait.
  fn offer_later(self: &Rc<Self>, space: &str, wait: Duration) {
    let (carbon, space) = (Rc::downgrade(self), space.to_string());
    let _ = self.after(wait, Box::new(move || carbon.upgrade().into_iter().for_each(|carbon| carbon.offer_held(&space))));
  }

  // Offer every followed space's held messages again, as after a commit, which may list a device.
  pub(crate) fn offer_all_held(self: &Rc<Self>) {
    let spaces: Vec<String> = self.nodes.spaces.followed_ids().into_iter().filter(|space| self.followed(space).is_some_and(|followed| !followed.held.borrow().is_empty())).collect();
    spaces.iter().for_each(|space| self.offer_held(space));
  }

  // Whether an identity on this device lists a device in its `[identity]`.
  fn lists(self: &Rc<Self>, identity: &str, device: &str) -> bool {
    let listed = || -> crate::Result<bool> {
      let held = local::acting(&self.system.store().reader()?, identity)?;
      let devices = self.named(&held.channel, "identity")?;
      self.read_tables(&devices, |db| Ok(!db.get("SELECT 1 AS present FROM device WHERE pubkey = ?", &[json!(device)])?.is_null()))
    };
    listed().unwrap_or(false)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ops::op::stamp;
  use crate::{Actor, Call, Routing};

  fn op(identity: &str, device: &str) -> Op {
    let actor = Actor { identity: Some(identity.into()), device: Some(device.into()) };
    stamp(&actor, &Call::from_key("POST [chat]/messages").unwrap(), &json!({"text": "hi"}), "v1", 1)
  }

  #[test]
  fn checks_the_hash_the_author_and_that_the_device_belongs_to_its_identity() {
    let first = "a1".repeat(32);
    let identity = id_of(&first);
    let forged = { let honest = op(&identity, &first); Op::new(honest.key, json!({"text": "bye"}), honest.author, honest.device, honest.version, honest.time, honest.hash) };
    let cases = [
      ("the first device", op(&identity, &first), first.clone(), false, Check::Holds),
      ("a device [identity] lists", op(&identity, "b2"), "b2".to_string(), true, Check::Holds),
      ("a device not yet listed", op(&identity, "b2"), "b2".to_string(), false, Check::Unlisted),
      ("another device as author", op(&identity, &first), "c3".to_string(), true, Check::Fails("device is not the p2panda author")),
      ("a forged input", forged, first.clone(), true, Check::Fails("hash")),
    ];
    for (named, op, author, listed, expected) in cases {
      assert_eq!(check(&author, &op, |_, _| listed), expected, "{named}");
    }
  }
}
