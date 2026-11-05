//! Stamping ops, and drawing from one while it applies. An op's hash covers
//! its envelope and its time, so it names the op on every replica; everything a
//! handler derives from it, such as ids and randomness, is drawn from that hash.

use std::cell::Cell;

use serde_json::{Value, json};

use crate::hash::hash;
use crate::{Actor, Call, Op};

// Stamp a call as an op at `time`.
pub fn stamp(actor: &Actor, call: &Call, input: &Value, version: usize, time: u64) -> Op {
  let key = call.key();
  let envelope = json!({"key": key, "input": input, "author": actor.identity, "device": actor.device, "version": version});
  let hash = hash(&json!({"envelope": envelope, "time": time}));
  Op { key, input: input.clone(), author: actor.identity.clone(), device: actor.device.clone(), version, time, hash }
}

// An op being applied, and how many values its handlers have drawn from it.
// Calls inside the op share it, so no two of them draw the same value.
pub struct Applying {
  pub op: Op,
  drawn: Cell<u64>,
}

impl Applying {
  pub fn new(op: Op) -> Applying {
    Applying { op, drawn: Cell::new(0) }
  }

  // The next hash drawn from the op, the source of its ids and randomness.
  pub fn draw(&self) -> String {
    let n = self.drawn.replace(self.drawn.get() + 1);
    hash(&json!({"op": self.op.hash, "n": n}))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn op(time: u64) -> Op {
    let actor = Actor { identity: Some("alice".into()), device: Some("pixel".into()) };
    stamp(&actor, &Call::from_key("POST [chat]/messages").unwrap(), &json!({"text": "hi"}), 1, time)
  }

  #[test]
  fn the_hash_covers_the_envelope_and_the_time() {
    assert_eq!(op(1).hash, op(1).hash);
    assert_ne!(op(1).hash, op(2).hash);
  }

  #[test]
  fn draws_never_repeat_within_an_op_and_repeat_across_replicas() {
    let (here, there) = (Applying::new(op(1)), Applying::new(op(1)));
    let drawn: Vec<String> = (0..3).map(|_| here.draw()).collect();
    assert_eq!(drawn.iter().collect::<std::collections::BTreeSet<_>>().len(), 3);
    assert_eq!(drawn, (0..3).map(|_| there.draw()).collect::<Vec<_>>());
  }
}
