//! A service's SQL: everything CarbonDB reads in it, the reach it allows it,
//! and what its statements mean for rewind.
//!
//! - `namespace` parses each statement, the one place SQL is parsed, and
//!   moves the names a service owns into its namespace.
//! - `lookup` finds the one row a statement reads, when it reads just one,
//!   confirmed by SQLite's query planner.
//! - `authorizer` refuses a statement that reaches outside its service's
//!   namespace and records what each statement read.
//! - `schema` checks a service's tables after its `init`.
//!
//! This module keeps what is worked out for each statement, once per
//! statement text and per space, in memory: the SQL as SQLite runs it, and
//! the row it looks up. The cache is cleared whenever SQLite's schema version
//! changes, so a trigger or an index added later is always seen; after a
//! restart each statement is worked out once more.
//!
//! sqlparser is used in `namespace`, which parses, and in `lookup`, which also
//! tokenizes a statement to number its parameters. A better way to read SQL
//! replaces only those two.

pub mod authorizer;
pub mod lookup;
pub mod namespace;
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
  known: RefCell<HashMap<(String, String), Rc<Analysed>>>,
}

impl Statements {
  // A service's statement as worked out the first time its text ran under
  // the schema this connection reads.
  pub(crate) fn get(&self, connection: &Connection, failure: &Failure, prefix: &str, sql: &str) -> Result<Rc<Analysed>> {
    let version = checked(failure, connection.prepare_cached("PRAGMA schema_version").and_then(|mut pragma| pragma.query_row([], |row| row.get(0))))?;
    if self.schema.replace(Some(version)) != Some(version) {
      self.known.borrow_mut().clear();
    }
    let key = (prefix.to_string(), sql.to_string());
    if let Some(known) = self.known.borrow().get(&key) {
      return Ok(Rc::clone(known));
    }
    let analysed = Rc::new(analyse(connection, prefix, sql)?);
    self.known.borrow_mut().insert(key, Rc::clone(&analysed));
    Ok(analysed)
  }
}

// A statement as SQLite runs it, and the row it looks up, if it reads just one.
#[derive(Debug, PartialEq)]
pub(crate) struct Analysed {
  pub sql: String,
  lookup: Option<Lookup>,
}

impl Analysed {
  // The row this run reads, from the values bound to it, when it reads just one.
  pub(crate) fn looked_up(&self, params: &[Value]) -> Option<Row> {
    self.lookup.as_ref()?.row(params)
  }
}

// Work a service's statement out: its namespaced SQL, and the row it looks up.
fn analyse(connection: &Connection, prefix: &str, sql: &str) -> Result<Analysed> {
  let statements = namespace::parse(sql)?;
  let renamed = namespace::rename(sql, &statements, prefix)?;
  let lookup = lookup::find(connection, sql, &statements, &renamed, prefix);
  Ok(Analysed { sql: renamed, lookup })
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use serde_json::json;

  const PREFIX: &str = "notes/db/";

  // A bare connection holding a service's namespaced table.
  pub(crate) fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(r#"CREATE TABLE "notes/db/note" (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID"#).unwrap();
    connection
  }

  #[test]
  fn a_statement_is_worked_out_once_until_the_schema_changes() {
    let (connection, statements, failure) = (connection(), Statements::default(), Failure::default());
    let sql = "SELECT text FROM note WHERE id = ?";
    let first = statements.get(&connection, &failure, PREFIX, sql).unwrap();
    assert!(Rc::ptr_eq(&first, &statements.get(&connection, &failure, PREFIX, sql).unwrap()), "a repeat run reuses its entry");
    connection.execute_batch(r#"CREATE TABLE "notes/db/tag" (name TEXT PRIMARY KEY) WITHOUT ROWID"#).unwrap();
    let again = statements.get(&connection, &failure, PREFIX, sql).unwrap();
    assert!(!Rc::ptr_eq(&first, &again), "a schema change replaces it");
    assert_eq!(*first, *again);
  }

  #[test]
  fn a_trigger_added_later_turns_a_row_read_back_into_a_read_of_the_whole_table() {
    let (connection, statements, failure) = (connection(), Statements::default(), Failure::default());
    let sql = "SELECT text FROM note WHERE id = ?";
    let params = [json!("a")];
    assert!(statements.get(&connection, &failure, PREFIX, sql).unwrap().looked_up(&params).is_some());
    connection.execute_batch(r#"CREATE TRIGGER "notes/db/kept" AFTER DELETE ON "notes/db/note" BEGIN SELECT 1; END"#).unwrap();
    assert_eq!(statements.get(&connection, &failure, PREFIX, sql).unwrap().looked_up(&params), None);
  }
}
