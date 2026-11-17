//! Stamping ops. An op's hash covers its envelope and its time, so it names
//! the op on every replica; everything a handler draws from it, such as ids and
//! randomness, comes from that hash.

use serde_json::{Value, json};

use crate::hash::hash;
use crate::{Actor, Call, Routing, Op};

// Stamp a call as an op at `time`, for the version of the service that handles it.
pub fn stamp(actor: &Actor, call: &Call, input: &Value, version: &str, time: u64) -> Op {
  let key = call.key();
  let hash = hash_of(&key, input, actor, version, time);
  Op::new(key, input.clone(), actor.identity.clone(), actor.device.clone(), version.to_string(), time, hash)
}

// The hash an op's envelope and time give it, which a received op's claimed hash must match.
pub fn hash_of(key: &str, input: &Value, actor: &Actor, version: &str, time: u64) -> String {
  let envelope = json!({"key": key, "input": input, "author": actor.identity, "device": actor.device, "version": version});
  hash(&json!({"envelope": envelope, "time": time}))
}

// Whether an op's hash is the one its envelope and time give it.
pub fn hash_holds(op: &Op) -> bool {
  hash_of(&op.key, &op.input, &actor(op), &op.version, op.time) == op.hash
}

// Who made an op, as its call acts.
pub fn actor(op: &Op) -> Actor {
  Actor { identity: op.author.clone(), device: op.device.clone() }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn op(time: u64) -> Op {
    let actor = Actor { identity: Some("alice".into()), device: Some("pixel".into()) };
    stamp(&actor, &Call::from_key("POST [chat]/messages").unwrap(), &json!({"text": "hi"}), "v1", time)
  }

  #[test]
  fn a_hash_holds_only_for_the_envelope_and_time_it_was_stamped_with() {
    let stamped = op(1);
    assert!(hash_holds(&stamped));
    let with = |input: Value, device: &str, time: u64| Op::new(stamped.key.clone(), input, stamped.author.clone(), Some(device.into()), stamped.version.clone(), time, stamped.hash.clone());
    let forged = [with(json!({"text": "hi"}), "pixel", 2), with(json!({"text": "hi"}), "other", 1), with(json!({"text": "bye"}), "pixel", 1)];
    assert!(forged.iter().all(|op| !hash_holds(op)));
  }

  #[test]
  fn the_hash_covers_the_envelope_and_the_time() {
    assert_eq!(op(1).hash, op(1).hash);
    assert_ne!(op(1).hash, op(2).hash);
    assert_eq!(actor(&op(1)), Actor { identity: Some("alice".into()), device: Some("pixel".into()) });
  }
}
