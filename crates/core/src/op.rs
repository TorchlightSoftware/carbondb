//! Stamping ops. An op's hash covers its envelope and its time, so it names
//! the op on every replica; everything a handler draws from it, such as ids and
//! randomness, comes from that hash.

use serde_json::{Value, json};

use crate::hash::hash;
use crate::{Actor, Call, Op};

// Stamp a call as an op at `time`, for the version of the service that handles it.
pub fn stamp(actor: &Actor, call: &Call, input: &Value, version: &str, time: u64) -> Op {
  let key = call.key();
  let envelope = json!({"key": key, "input": input, "author": actor.identity, "device": actor.device, "version": version});
  let hash = hash(&json!({"envelope": envelope, "time": time}));
  Op::new(key, input.clone(), actor.identity.clone(), actor.device.clone(), version.to_string(), time, hash)
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
  fn the_hash_covers_the_envelope_and_the_time() {
    assert_eq!(op(1).hash, op(1).hash);
    assert_ne!(op(1).hash, op(2).hash);
    assert_eq!(actor(&op(1)), Actor { identity: Some("alice".into()), device: Some("pixel".into()) });
  }
}
