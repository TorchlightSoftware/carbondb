//! The OpWriter: writes every op a space receives to its op log, local calls
//! now and synced ops once replication lands. An op is written as it arrives,
//! unless another op is applying: that op's transaction holds the file, so the
//! new one is held here and written as soon as the turn ends, before the next
//! op applies.
//!
//! The OpWriter only receives. Which op applies next is read from the log: the
//! pending ops, the op log minus results.

use std::cell::RefCell;

use crate::error::Result;
use crate::store::Store;
use crate::{Op, oplog};

#[derive(Default)]
pub struct OpWriter {
  // Ops that arrived while another op applied, in the order they arrived.
  held: RefCell<Vec<Op>>,
}

impl OpWriter {
  // Write an op to the log now, or hold it until the op applying is done.
  pub fn write(&self, store: &Store, applying: bool, op: Op) -> Result<()> {
    if applying {
      self.held.borrow_mut().push(op);
      return Ok(());
    }
    oplog::append(store, &op)
  }

  // Write the ops held while the last op applied.
  pub fn write_held(&self, store: &Store) -> Result<()> {
    let held = std::mem::take(&mut *self.held.borrow_mut());
    held.iter().try_for_each(|op| oplog::append(store, op))
  }

  // Let go of the held ops without writing them, as a stopped space does.
  pub fn discard(&self) {
    self.held.borrow_mut().clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn op(hash: &str, time: u64) -> Op {
    Op::new("POST [notes]/run".into(), json!({}), None, None, "v".into(), time, hash.into())
  }

  fn logged(store: &Store) -> Vec<String> {
    oplog::entries(store).unwrap().into_iter().map(|op| op.hash).collect()
  }

  #[test]
  fn writes_an_op_at_once_unless_another_applies_then_when_its_turn_ends() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("s.db")).unwrap();
    store.run_batch(oplog::SCHEMA).unwrap();
    let writer = OpWriter::default();
    writer.write(&store, false, op("a", 1)).unwrap();
    writer.write(&store, true, op("b", 2)).unwrap();
    writer.write(&store, true, op("c", 3)).unwrap();
    assert_eq!(logged(&store), ["a"]);
    writer.write_held(&store).unwrap();
    assert_eq!(logged(&store), ["a", "b", "c"]);
    writer.write(&store, true, op("d", 4)).unwrap();
    writer.discard();
    writer.write_held(&store).unwrap();
    assert_eq!(logged(&store), ["a", "b", "c"]);
  }
}
