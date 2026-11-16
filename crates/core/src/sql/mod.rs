//! A service's SQL: what CarbonDB reads in it, the reach it allows it, and
//! what its statements mean for rewind.
//!
//! - `lookup` finds the one row a statement reads, when it reads just one,
//!   confirmed by SQLite's query planner. It is the one place SQL is parsed,
//!   with sqlparser; a statement it cannot read runs as written and reads its
//!   whole tables.
//! - `authorizer` refuses a statement that reaches the core's own tables or
//!   another file, and records what each statement read.
//! - `schema` checks a service's tables after its `setup`.
//!
//! This module keeps what is worked out for each statement, once per statement
//! text and per space, in memory: the row it looks up. The cache is cleared
//! whenever SQLite's schema version changes, so a trigger or an index added
//! later is always seen; after a restart each statement is worked out once
//! more.

pub mod authorizer;
pub mod lookup;
pub mod schema;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rusqlite::Connection;
use serde_json::Value;

use crate::error::Result;
use crate::store::changeset::Row;
use crate::store::{Failure, checked};
use lookup::Lookup;

// The statements a space's services have run, under the schema version they were worked out for.
#[derive(Default)]
pub(crate) struct Statements {
  schema: Cell<Option<i64>>,
  known: RefCell<HashMap<String, Rc<Analysed>>>,
}

impl Statements {
  // A service's statement as worked out the first time its text ran under
  // the schema this connection reads.
  pub(crate) fn get(&self, connection: &Connection, failure: &Failure, sql: &str) -> Result<Rc<Analysed>> {
    let version = checked(failure, connection.prepare_cached("PRAGMA schema_version").and_then(|mut pragma| pragma.query_row([], |row| row.get(0))))?;
    if self.schema.replace(Some(version)) != Some(version) {
      self.known.borrow_mut().clear();
    }
    if let Some(known) = self.known.borrow().get(sql) {
      return Ok(Rc::clone(known));
    }
    let analysed = Rc::new(Analysed { lookup: lookup::find(connection, sql) });
    self.known.borrow_mut().insert(sql.to_string(), Rc::clone(&analysed));
    Ok(analysed)
  }
}

// The row a statement looks up, if it reads just one.
#[derive(Debug, PartialEq)]
pub(crate) struct Analysed {
  lookup: Option<Lookup>,
}

impl Analysed {
  // The row this run reads, from the values bound to it, when it reads just one.
  pub(crate) fn looked_up(&self, params: &[Value]) -> Option<Row> {
    self.lookup.as_ref()?.row(params)
  }
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use serde_json::json;

  // A bare connection holding a service's table.
  pub(crate) fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID").unwrap();
    connection
  }

  #[test]
  fn a_statement_is_worked_out_once_until_the_schema_changes() {
    let (connection, statements, failure) = (connection(), Statements::default(), Failure::default());
    let sql = "SELECT text FROM note WHERE id = ?";
    let first = statements.get(&connection, &failure, sql).unwrap();
    assert!(Rc::ptr_eq(&first, &statements.get(&connection, &failure, sql).unwrap()), "a repeat run reuses its entry");
    connection.execute_batch("CREATE TABLE tag (name TEXT PRIMARY KEY) WITHOUT ROWID").unwrap();
    let again = statements.get(&connection, &failure, sql).unwrap();
    assert!(!Rc::ptr_eq(&first, &again), "a schema change replaces it");
    assert_eq!(*first, *again);
  }

  #[test]
  fn a_trigger_added_later_turns_a_row_read_back_into_a_read_of_the_whole_table() {
    let (connection, statements, failure) = (connection(), Statements::default(), Failure::default());
    let sql = "SELECT text FROM note WHERE id = ?";
    let params = [json!("a")];
    assert!(statements.get(&connection, &failure, sql).unwrap().looked_up(&params).is_some());
    connection.execute_batch("CREATE TRIGGER kept AFTER DELETE ON note BEGIN SELECT 1; END").unwrap();
    assert_eq!(statements.get(&connection, &failure, sql).unwrap().looked_up(&params), None);
  }
}
