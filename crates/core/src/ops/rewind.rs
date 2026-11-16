//! Rewind: what a late op does to the ops after it. An op is late when it
//! sorts before ops this device has already applied; the clock decides that,
//! never the read set.
//!
//! A late op is first tried in place, on today's state. It keeps that run
//! when no later op wrote a row it read, no later op read a row it wrote, and
//! no later op wrote a row it wrote: it then saw exactly the state at its own
//! place and changed nothing a later op depends on, so the result is the same
//! as applying it in order. Otherwise its run rolls back and the space
//! rewinds. Writes are known by row, from the changeset. Reads are known by
//! row when a statement looks one row up by its whole key (`sql::lookup`), and
//! otherwise by table, which reads every row of it. So ops inserting different
//! rows into one table apply in place, and so does an insert before an edit
//! that looked up another row by its key: a handler that needs a value to be
//! unique checks with a query, which is a read, never with a constraint.
//!
//! A space runs one service, so a rewind undoes every later op that touched
//! any of its tables, newest first. An op that read and wrote nothing cannot
//! have depended on the late op, nor changed what it sees, so it stays. The
//! undone ops lose their results and are pending again, so the late op
//! applies at its place, then they re-run in order, each tried in place like
//! any late op.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::error::Result;
use crate::ops::oplog::ORDER;
use crate::sql::authorizer::Read;
use crate::store::changeset::{Row, rows};
use crate::{Db, Op};

// An op's read set and write set: the tables and rows it read, and the rows its changeset changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sets {
  pub read_set: BTreeSet<Read>,
  pub write_set: BTreeSet<Row>,
}

impl Sets {
  pub fn of(read_set: &[Read], changeset: &[u8]) -> Result<Sets> {
    Ok(Sets { read_set: read_set.iter().cloned().collect(), write_set: rows(changeset)?.into_iter().collect() })
  }

  // The tables it wrote.
  pub fn written(&self) -> BTreeSet<String> {
    self.write_set.iter().map(|row| row.table.clone()).collect()
  }

  // Whether it read a row: the row itself, or its whole table.
  fn reads(&self, row: &Row) -> bool {
    self.read_set.contains(&Read::Row(row.clone())) || self.read_set.contains(&Read::Table(row.table.clone()))
  }

  // Whether it read or wrote anything.
  fn touched_any(&self) -> bool {
    !self.read_set.is_empty() || !self.write_set.is_empty()
  }
}

// An op this device has a result for, as rewind sees it: its sets and the changeset that undoes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
  pub hash: String,
  pub sets: Sets,
  pub changeset: Vec<u8>,
}

// Whether a late op's in-place run gives the same result as applying it at its place.
pub fn fits_in_place(late: &Sets, later: &[Recorded]) -> bool {
  later.iter().all(|op| independent(late, &op.sets))
}

// Whether two ops' runs leave each other be, whichever runs first: neither
// read a row the other wrote, and they wrote no row in common.
fn independent(one: &Sets, other: &Sets) -> bool {
  !one.write_set.iter().any(|row| other.reads(row)) && !other.write_set.iter().any(|row| one.reads(row)) && one.write_set.is_disjoint(&other.write_set)
}

// The later ops a rewind undoes, newest first: those that touched a service
// the late op touched, widened to every service they touched in turn.
pub fn undo_set(later: &[Recorded]) -> Vec<&Recorded> {
  later.iter().filter(|op| op.sets.touched_any()).rev().collect()
}

// The ops after `op` in the shared order that this device has run, oldest first.
pub fn after(db: &dyn Db, op: &Op) -> Result<Vec<Recorded>> {
  let run = "SELECT o.hash, o.time, o.device, o.key, r.outcome, r.read_set, r.changeset FROM carbon_oplog o JOIN carbon_result r USING (hash)";
  let sql = format!("SELECT * FROM ({run}) WHERE (time, COALESCE(device, ''), hash) > (?, ?, ?) {ORDER}");
  let rows = db.all(&sql, &[json!(op.time), json!(op.device.clone().unwrap_or_default()), json!(op.hash)])?;
  rows.into_iter().map(to_recorded).collect()
}

fn to_recorded(row: Value) -> Result<Recorded> {
  let read_set: Vec<Read> = serde_json::from_str(row["read_set"].as_str().unwrap_or("[]"))?;
  // A result's changeset reads back through the JSON binding as an array of bytes.
  let changeset: Vec<u8> = serde_json::from_value(row["changeset"].clone())?;
  Ok(Recorded { hash: row["hash"].as_str().unwrap_or_default().to_string(), sets: Sets::of(&read_set, &changeset)?, changeset })
}

#[cfg(test)]
mod tests {
  use super::*;

  // A read names a row as `table@key`, or a table alone for all of it; a
  // write names a row as `table@key`, or a table alone for some row of it.
  fn sets(read: &[&str], write: &[&str]) -> Sets {
    let row = |named: &str, otherwise: &str| {
      let (table, key) = named.split_once('@').unwrap_or((named, otherwise));
      Row { table: table.into(), key: key.into() }
    };
    let read_one = |named: &&str| match named.contains('@') {
      true => Read::Row(row(named, "")),
      false => Read::Table(named.to_string()),
    };
    Sets { read_set: read.iter().map(read_one).collect(), write_set: write.iter().map(|named| row(named, "some")).collect() }
  }

  fn recorded(hash: &str, read: &[&str], write: &[&str]) -> Recorded {
    Recorded { hash: hash.into(), sets: sets(read, write), changeset: vec![] }
  }

  #[test]
  fn a_late_op_fits_in_place_only_when_neither_it_nor_a_later_op_depends_on_the_other() {
    let cases = [
      ("disjoint tables", sets(&["notes/db/note"], &["notes/db/note"]), recorded("x", &["todo/db/item"], &["todo/db/item"]), true),
      ("both only read the same table", sets(&["notes/db/note"], &[]), recorded("x", &["notes/db/note"], &[]), true),
      ("a later op wrote what it read", sets(&["notes/db/note"], &[]), recorded("x", &[], &["notes/db/note"]), false),
      ("a later op read what it wrote", sets(&[], &["notes/db/note"]), recorded("x", &["notes/db/note"], &[]), false),
      ("both wrote one row", sets(&[], &["notes/db/note@a"]), recorded("x", &[], &["notes/db/note@a"]), false),
      ("both wrote different rows of one table, reading none", sets(&[], &["notes/db/note@a"]), recorded("x", &[], &["notes/db/note@b"]), true),
      ("a later op read the table it wrote a different row of", sets(&[], &["notes/db/note@a"]), recorded("x", &["notes/db/note"], &["notes/db/note@b"]), false),
      ("it read the table a later op wrote a different row of", sets(&["notes/db/note"], &["notes/db/note@a"]), recorded("x", &[], &["notes/db/note@b"]), false),
      ("a later op read another row by its key", sets(&[], &["notes/db/note@a"]), recorded("x", &["notes/db/note@b"], &["notes/db/note@b"]), true),
      ("a later op read the row it wrote by its key", sets(&[], &["notes/db/note@a"]), recorded("x", &["notes/db/note@a"], &[]), false),
      ("it read by its key a row a later op wrote", sets(&["notes/db/note@b"], &[]), recorded("x", &[], &["notes/db/note@b"]), false),
      ("it read by its key a row no later op wrote", sets(&["notes/db/note@b"], &[]), recorded("x", &[], &["notes/db/note@c"]), true),
      ("another table of the same service", sets(&["notes/db/note"], &["notes/db/note"]), recorded("x", &["notes/db/tag"], &["notes/db/tag"]), true),
    ];
    for (name, late, later, expected) in cases {
      assert_eq!(fits_in_place(&late, &[later]), expected, "{name}");
    }
  }

  #[test]
  fn a_rewind_undoes_every_later_op_that_touched_a_table_newest_first() {
    let later = [recorded("read", &["note"], &[]), recorded("nothing", &[], &[]), recorded("wrote", &[], &["tag"])];
    let undone: Vec<&str> = undo_set(&later).into_iter().map(|op| op.hash.as_str()).collect();
    assert_eq!(undone, ["wrote", "read"]);
  }
}
