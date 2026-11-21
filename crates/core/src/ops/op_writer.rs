//! The OpWriter: writes a service's pointer to each op addressed to it, as
//! the op is logged in its channel's log, and the rows of it each `callsync`
//! read by another service's op read (`ops::reads`). Each is written at once,
//! unless one of the service's ops is applying: that op's transaction holds the
//! service's file, so it is held here and written as soon as the turn ends,
//! before the next op applies.
//!
//! The OpWriter only receives. Which op applies next is read from the
//! pointers: the pending ops, the pointers minus results.

use std::cell::RefCell;

use crate::error::Result;
use crate::ops::pointers::{self, Pointer};
use crate::ops::reads::{self, Reading};
use crate::store::Store;
use crate::store::changeset::Row;

#[derive(Default)]
pub struct OpWriter {
  // Pointers that arrived while another op applied, in the order they arrived.
  held: RefCell<Vec<Pointer>>,
  // Reads made of the service while one of its ops applied, by their readers.
  held_reads: RefCell<Vec<(Vec<Row>, Reading)>>,
}

impl OpWriter {
  // Write a pointer now, or hold it until the op applying is done.
  pub fn write(&self, store: &Store, applying: bool, pointer: Pointer) -> Result<()> {
    if applying {
      self.held.borrow_mut().push(pointer);
      return Ok(());
    }
    pointers::write(store, &pointer)
  }

  // Write what another service's op read now, or hold it until the op applying is done.
  pub fn record_reads(&self, store: &Store, applying: bool, rows: Vec<Row>, reading: Reading) -> Result<()> {
    if applying {
      self.held_reads.borrow_mut().push((rows, reading));
      return Ok(());
    }
    reads::record(store, &rows, &reading)
  }

  // Whether a pointer to an op placed before a place is held.
  pub fn holds_before(&self, place: &Pointer) -> bool {
    self.held.borrow().iter().any(|pointer| pointer.before(place))
  }

  // Write the pointers and reads held while the last op applied.
  pub fn write_held(&self, store: &Store) -> Result<()> {
    let held = std::mem::take(&mut *self.held.borrow_mut());
    held.iter().try_for_each(|pointer| pointers::write(store, pointer))?;
    let held_reads = std::mem::take(&mut *self.held_reads.borrow_mut());
    held_reads.iter().try_for_each(|(rows, reading)| reads::record(store, rows, reading))
  }

  // Let go of what is held without writing it, as a stopped service does.
  pub fn discard(&self) {
    self.held.borrow_mut().clear();
    self.held_reads.borrow_mut().clear();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pointer(hash: &str, time: u64) -> Pointer {
    Pointer { hash: hash.into(), time, device: None }
  }

  #[test]
  fn writes_a_pointer_at_once_unless_an_op_applies_then_when_its_turn_ends() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("s.db")).unwrap();
    store.migrate(crate::channels::service::MIGRATIONS).unwrap();
    let writer = OpWriter::default();
    writer.write(&store, false, pointer("a", 1)).unwrap();
    writer.write(&store, true, pointer("b", 2)).unwrap();
    writer.write(&store, true, pointer("c", 3)).unwrap();
    assert_eq!(pointers::hashes(&store).unwrap(), ["a"]);
    writer.write_held(&store).unwrap();
    assert_eq!(pointers::hashes(&store).unwrap(), ["a", "b", "c"]);
    writer.write(&store, true, pointer("d", 4)).unwrap();
    writer.discard();
    writer.write_held(&store).unwrap();
    writer.write(&store, false, pointer("a", 1)).unwrap();
    assert_eq!(pointers::hashes(&store).unwrap(), ["a", "b", "c"]);
  }
}
