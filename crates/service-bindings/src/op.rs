//! An op as a handler sees it: a service call that writes, stamped with who
//! made it, when, and the version of the service that handles it. Its hash
//! names it on every replica, and its ids and randomness are drawn from that
//! hash, so every replica draws the same values in the same order.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::hash::hash;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Op {
  pub key: String,
  pub input: Value,
  pub author: Option<String>,
  pub device: Option<String>,
  // The hash of the package that handles the call.
  pub version: String,
  pub time: u64,
  pub hash: String,
  // How many values the op's handlers have drawn. Clones share it, so the
  // calls inside one op never draw the same value.
  #[serde(skip)]
  drawn: Arc<AtomicU64>,
}

impl PartialEq for Op {
  fn eq(&self, other: &Op) -> bool {
    self.hash == other.hash
  }
}

impl Op {
  pub fn new(key: String, input: Value, author: Option<String>, device: Option<String>, version: String, time: u64, hash: String) -> Op {
    Op { key, input, author, device, version, time, hash, drawn: Arc::default() }
  }

  // An id for something the op creates: 32 hex characters.
  pub fn id(&self) -> String {
    self.draw()[..32].to_string()
  }

  // A number in [0, 1) with 52 bits of precision.
  pub fn random(&self) -> f64 {
    let bits = u64::from_str_radix(&self.draw()[..13], 16).expect("a hash is hex");
    bits as f64 / (1u64 << 52) as f64
  }

  fn draw(&self) -> String {
    let n = self.drawn.fetch_add(1, Ordering::SeqCst);
    hash(&json!({"op": self.hash, "n": n}))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn op() -> Op {
    Op::new("POST [chat]/messages".into(), json!({}), None, None, "v".into(), 1, "h".into())
  }

  #[test]
  fn draws_never_repeat_within_an_op_and_repeat_across_replicas() {
    let (here, there) = (op(), op());
    let shared = here.clone();
    let drawn = [here.id(), shared.id(), here.id()];
    assert_eq!(drawn.iter().collect::<std::collections::BTreeSet<_>>().len(), 3);
    assert_eq!(drawn, [there.id(), there.id(), there.id()]);
    assert!(drawn.iter().all(|id| id.len() == 32));
  }

  #[test]
  fn random_values_fall_in_the_unit_interval() {
    let op = op();
    assert!((0..100).map(|_| op.random()).all(|value| (0.0..1.0).contains(&value)));
  }
}
