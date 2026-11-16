//! The OpWriter: writes every op a space receives to its op log, as a log row:
//! local calls, and rows received from other devices. A row is written as it
//! arrives, unless another op is applying: that op's transaction holds the
//! file, so the new row is held here and written as soon as the turn ends,
//! before the next op applies.
//!
//! The OpWriter only receives. Which op applies next is read from the log: the
//! pending ops, the op log minus results.

use std::cell::RefCell;

use crate::error::Result;
use crate::ops::oplog::{self, LogRow};
use crate::store::Store;

#[derive(Default)]
pub struct OpWriter {
  // Rows that arrived while another op applied, in the order they arrived.
  held: RefCell<Vec<LogRow>>,
}

impl OpWriter {
  // Write a row to the log now, or hold it until the op applying is done.
  pub fn write(&self, store: &Store, applying: bool, row: LogRow) -> Result<()> {
    if applying {
      self.held.borrow_mut().push(row);
      return Ok(());
    }
    oplog::append(store, &row)
  }

  // Write the rows held while the last op applied.
  pub fn write_held(&self, store: &Store) -> Result<()> {
    let held = std::mem::take(&mut *self.held.borrow_mut());
    held.iter().try_for_each(|row| oplog::append(store, row))
  }

  // Let go of the held rows without writing them, as a stopped space does.
  pub fn discard(&self) {
    self.held.borrow_mut().clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Op;
  use crate::ops::effects::Made;
  use serde_json::json;

  fn row(hash: &str, time: u64) -> LogRow {
    LogRow::new(Op::new("POST [notes]/run".into(), json!({}), None, None, "v".into(), time, hash.into()))
  }

  fn logged(store: &Store) -> Vec<String> {
    oplog::entries(store).unwrap().into_iter().map(|op| op.hash).collect()
  }

  #[test]
  fn writes_an_op_at_once_unless_another_applies_then_when_its_turn_ends() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("s.db")).unwrap();
    store.migrate(crate::channels::service::MIGRATIONS).unwrap();
    let writer = OpWriter::default();
    writer.write(&store, false, row("a", 1)).unwrap();
    writer.write(&store, true, row("b", 2)).unwrap();
    writer.write(&store, true, row("c", 3)).unwrap();
    assert_eq!(logged(&store), ["a"]);
    writer.write_held(&store).unwrap();
    assert_eq!(logged(&store), ["a", "b", "c"]);
    writer.write(&store, true, row("d", 4)).unwrap();
    writer.discard();
    writer.write_held(&store).unwrap();
    assert_eq!(logged(&store), ["a", "b", "c"]);
  }

  #[test]
  fn drops_an_op_already_logged_and_keeps_a_received_row_with_its_effects() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("s.db")).unwrap();
    store.migrate(crate::channels::service::MIGRATIONS).unwrap();
    let writer = OpWriter::default();
    let made = Made { name: "call".into(), input: json!({"key": "GET [tag]/tags"}), output: Ok(json!(["red"])) };
    let received = LogRow { effects: Some(vec![made.clone()]), ..row("l", 1) };
    writer.write(&store, false, received.clone()).unwrap();
    writer.write(&store, false, row("l", 1)).unwrap();
    assert_eq!(oplog::rows(&store).unwrap(), vec![received]);
    assert_eq!(oplog::effects(&store, "l").unwrap(), Some(vec![made]));
  }
}
