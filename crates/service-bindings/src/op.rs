//! An op as a handler sees it: a service call that writes, stamped with who
//! made it, when, and the version of the service that handles it. Its hash
//! names it on every replica, and its ids and randomness are drawn from that
//! hash, so every replica draws the same values in the same order.
//!
//! The core counts an op's draws, so that every call within one op draws
//! from one sequence. A service running as a component asks the core for
//! each draw instead (`Op::drawn_by`), since a call within the op may run in
//! another instance.

use std::fmt;
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
  // Where its draws come from. Clones share it, so the calls inside one op
  // never draw the same value.
  #[serde(skip)]
  draws: Draws,
}

// The next value drawn from an op, given its hash: a 64-character hex hash.
#[derive(Clone)]
pub struct Draws(Arc<dyn Fn(&str) -> String + Send + Sync>);

// Counted here: the nth draw is the hash of the op's hash and n.
impl Default for Draws {
  fn default() -> Draws {
    let drawn = AtomicU64::new(0);
    Draws(Arc::new(move |op| hash(&json!({"op": op, "n": drawn.fetch_add(1, Ordering::SeqCst)}))))
  }
}

impl fmt::Debug for Draws {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str("Draws")
  }
}

impl PartialEq for Op {
  fn eq(&self, other: &Op) -> bool {
    self.hash == other.hash
  }
}

impl Op {
  pub fn new(key: String, input: Value, author: Option<String>, device: Option<String>, version: String, time: u64, hash: String) -> Op {
    Op { key, input, author, device, version, time, hash, draws: Draws::default() }
  }

  // The same op, its draws asked of `draw` rather than counted here.
  pub fn drawn_by(self, draw: impl Fn() -> String + Send + Sync + 'static) -> Op {
    Op { draws: Draws(Arc::new(move |_| draw())), ..self }
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

  // The next value drawn: a 64-character hex hash.
  pub fn draw(&self) -> String {
    (self.draws.0)(&self.hash)
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
  fn an_op_drawn_by_another_takes_its_values_from_it() {
    let counted = op();
    let drawn = [counted.draw(), counted.draw()];
    let source = op();
    let asked = op().drawn_by(move || source.draw());
    assert_eq!([asked.draw(), asked.draw()], drawn);
    assert_eq!(asked.id(), counted.id());
  }

  #[test]
  fn random_values_fall_in_the_unit_interval() {
    let op = op();
    assert!((0..100).map(|_| op.random()).all(|value| (0.0..1.0).contains(&value)));
  }
}
