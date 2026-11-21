//! A read through `callsync`: another service's rows as they stood at the
//! reader's place in the shared order.
//!
//! A store applies ops in the order they reach it, not in their places, so no
//! one commit need hold every row as it stood at a place. One row's history
//! is in order, though: an op that writes a row a later op already read or
//! wrote never fits in place, and rewinds (`ops::rewind`). So a row as of a
//! place is the row as it is now, with the changes the ops placed after it
//! made to it undone, newest first (`store::changeset::row_as_of`), from the
//! changesets rewinds keep (`ops::reads`). Each statement must look up one
//! row by its whole key (`sql::lookup`), which is refused the same on every
//! replica otherwise; it then runs unchanged on the store's scratch
//! connection, against that row alone (`store::scratch`). Why rows are
//! rebuilt rather than read from a kept WAL: docs/wal-vs-changesets.md.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use serde_json::Value;

use crate::Db;
use crate::error::{Result, ServiceError, bad_request};
use crate::ops::reads::{self, Reading};
use crate::sql::authorizer::{Read, unrecorded};
use crate::sql::call_window::CallWindow;
use crate::store::Reader;
use crate::store::changeset::{Row, Values, change_to, row_as_of};
use crate::store::scratch::{Declared, Scratch};

// A read as of the place of the op reading: the store as it is now, in one
// read, the rows it read, and where they are recorded as read.
pub struct AsOf {
  now: Reader,
  scratch: Rc<Scratch>,
  reading: Reading,
  read: RefCell<BTreeSet<Row>>,
}

impl AsOf {
  pub(crate) fn over(now: Reader, scratch: Rc<Scratch>, reading: Reading) -> AsOf {
    AsOf { now, scratch, reading, read: RefCell::default() }
  }

  // The op reading, by its place, and the store it applies in.
  pub(crate) fn reading(&self) -> &Reading {
    &self.reading
  }

  // A service's statements, each reading its row as it stood.
  pub fn scoped(&self, window: CallWindow) -> ReadAsOf<'_> {
    ReadAsOf { as_of: self, window }
  }

  // End the read and return the rows it read.
  pub fn end(&self) -> Result<Vec<Row>> {
    self.now.end()?;
    Ok(std::mem::take(&mut *self.read.borrow_mut()).into_iter().collect())
  }

  // The row a statement looks up by its whole key, or a refusal.
  fn looked_up(&self, sql: &str, params: &[Value]) -> Result<Row> {
    let now = &self.now;
    let analysed = unrecorded(&now.access, || now.statements.get(&now.connection, &now.failure, sql))?;
    analysed.looked_up(params).ok_or_else(|| by_id_only(sql))
  }

  // A row's table, and the row as it stood at the reader's place.
  fn stood(&self, row: &Row) -> Result<(Declared, Option<Values>)> {
    let now = &self.now;
    unrecorded(&now.access, || {
      let table = Declared::of(now, &row.table)?;
      let current = table.row(&now.connection, &now.failure, row)?;
      let later = reads::later_changesets(now, row, &self.reading.place)?;
      let changes = later.iter().map(|changeset| change_to(changeset, row)).collect::<Result<Vec<_>>>()?;
      Ok((table, row_as_of(current, changes.into_iter().flatten())))
    })
  }
}

// A read as of a place as a service sees it: each statement inside its call window.
pub struct ReadAsOf<'a> {
  as_of: &'a AsOf,
  window: CallWindow,
}

impl ReadAsOf<'_> {
  // Run a statement against the one row it looks up, as that row stood, and
  // keep the row as read; refused if it read anything else.
  fn read(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    let row = self.as_of.looked_up(sql, params)?;
    let (table, stood) = self.as_of.stood(&row)?;
    let scratch = &self.as_of.scratch;
    scratch.hold(&table, stood.as_ref())?;
    let (rows, read) = scratch.read(&self.window, &row, sql, params);
    if read != [Read::Row(row.clone())] {
      return Err(by_id_only(sql));
    }
    self.as_of.read.borrow_mut().insert(row);
    rows
  }
}

impl Db for ReadAsOf<'_> {
  fn run(&self, sql: &str, _params: &[Value]) -> Result<u64> {
    Err(bad_request(format!("A read through callsync never writes: {sql}")))
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    self.read(sql, params)
  }
}

// A read through `callsync` that is not one row looked up by its whole key.
fn by_id_only(sql: &str) -> ServiceError {
  bad_request(format!("A read through callsync looks up one row of another service by its whole key, and this one reads more: {sql}"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ops::reads::tests::{MEMBER, apply, place, store};
  use crate::store::Store;
  use serde_json::json;

  // What a `callsync` read made by an op at a time answers.
  fn read_at(store: &Store, time: u64, sql: &str, params: &[Value]) -> Vec<Value> {
    let as_of = store.reader_as_of(Reading { place: place("reader", time), store: "books".into() }).unwrap();
    let rows = as_of.scoped(CallWindow { service: true, op: None }).all(sql, params).unwrap();
    as_of.end().unwrap();
    rows
  }

  // Ann holds the inviter role; Dave adds Bob at 20 on a device that is
  // offline, and revokes Ann's role at 30 from one that is not, so the
  // revocation applies here first. No commit holds Ann as an inviter and Bob
  // as a member, yet a read at 25 finds both.
  #[test]
  fn a_row_reads_as_it_stood_at_a_place_whatever_order_its_ops_applied_in() {
    let (_directory, store) = store(MEMBER);
    apply(&store, &place("grant", 10), "INSERT INTO member VALUES ('ann', 'inviter')");
    apply(&store, &place("revoke", 30), "UPDATE member SET role = NULL WHERE id = 'ann'");
    apply(&store, &place("add bob", 20), "INSERT INTO member VALUES ('bob', NULL)");
    let role = |role: Value| vec![json!({"role": role})];
    let cases = [
      ("before anything", 5, vec![], vec![]),
      ("after the grant", 15, role(json!("inviter")), vec![]),
      ("between Bob's addition and the revocation", 25, role(json!("inviter")), role(json!(null))),
      ("after the revocation", 35, role(json!(null)), role(json!(null))),
    ];
    for (name, time, ann, bob) in cases {
      let sql = "SELECT role FROM member WHERE id = ?";
      assert_eq!((read_at(&store, time, sql, &[json!("ann")]), read_at(&store, time, sql, &[json!("bob")])), (ann, bob), "{name}");
    }
  }

  // A row read at 15, between an op at 10 and one at 20, answers as the
  // store's table would have then, however the table is declared.
  #[test]
  fn a_row_reads_as_it_stood_whatever_its_table_declares() {
    let member = "CREATE TABLE member (id TEXT PRIMARY KEY, name TEXT COLLATE NOCASE, role TEXT) WITHOUT ROWID";
    let add_ann = "INSERT INTO member (id, name, role) VALUES ('ann', 'Ann', 'member')";
    let promote = "UPDATE member SET role = 'inviter', name = 'Annie' WHERE id = 'ann'";
    let note = "ALTER TABLE member ADD COLUMN note TEXT DEFAULT 'none'";
    let by_id = |sql: &'static str| (sql, vec![json!("ann")]);
    let cases = [
      ("a key declared out of column order", "CREATE TABLE grant (person TEXT, role TEXT, since INTEGER, PRIMARY KEY (role, person)) WITHOUT ROWID", "INSERT INTO grant VALUES ('ann', 'inviter', 1)", "UPDATE grant SET since = 2", None, ("SELECT since FROM grant WHERE role = ? AND person = ?", vec![json!("inviter"), json!("ann")]), json!({"since": 1})),
      ("a column's collation", member, add_ann, promote, None, by_id("SELECT name = 'ANN' AS same FROM member WHERE id = ?"), json!({"same": 1})),
      ("a generated column", "CREATE TABLE member (id TEXT PRIMARY KEY, name TEXT, greeting TEXT GENERATED ALWAYS AS ('Hi ' || name), role TEXT) WITHOUT ROWID", add_ann, promote, None, by_id("SELECT greeting, role FROM member WHERE id = ?"), json!({"greeting": "Hi Ann", "role": "member"})),
      ("a column added since, on a row updated since", member, add_ann, promote, Some(note), by_id("SELECT role, note FROM member WHERE id = ?"), json!({"role": "member", "note": "none"})),
      ("a column added since, on a row deleted since", member, add_ann, "DELETE FROM member", Some(note), by_id("SELECT role, note FROM member WHERE id = ?"), json!({"role": "member", "note": "none"})),
    ];
    for (name, table, first, second, migration, (sql, params), expected) in cases {
      let (_directory, store) = store(table);
      apply(&store, &place("first", 10), first);
      apply(&store, &place("second", 20), second);
      if let Some(migration) = migration {
        store.run_batch(migration).unwrap();
      }
      assert_eq!(read_at(&store, 15, sql, &params), [expected], "{name}");
    }
  }
}
