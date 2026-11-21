//! A scratch connection: a store's tables as the store declares them, in
//! memory, each holding one row at a time, for a service's statement to read
//! that row exactly as it would read the store.
//!
//! A read through `callsync` reads a row as it stood at a place
//! (`store::as_of`). That row is rebuilt from changesets and is in no table of
//! the store, so the statement runs here instead, unchanged, against a table
//! declared from the store's own `CREATE TABLE` text: its columns keep their
//! type affinity and collation, so the statement answers as it would have
//! then. A table is declared again when that text changes. Foreign keys stay
//! off, as for any new connection: the rows they name are not here.

use std::cell::RefCell;
use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Value, json};

use crate::Db;
use crate::error::{Result, sqlite};
use crate::sql::authorizer::{Read, Shared, start_reading, stop_reading};
use crate::sql::call_window::{CallWindow, guard, within};
use crate::store::changeset::{Row, Values};
use crate::store::{Failure, all, batch, checked, to_sql};

// A table as the store declares it: the statement that created it, its
// columns in order, leaving out generated ones, which no changeset holds,
// and its key's columns in key order.
pub(crate) struct Declared {
  pub(crate) name: String,
  create: String,
  columns: Vec<String>,
  key: Vec<String>,
}

impl Declared {
  // A table as a store's file declares it now.
  pub(crate) fn of(db: &dyn Db, table: &str) -> Result<Declared> {
    let create = db.get("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?", &[json!(table)])?;
    let names = |sql: &str| -> Result<Vec<String>> { Ok(db.all(sql, &[json!(table)])?.iter().map(|column| column["name"].as_str().unwrap_or_default().to_string()).collect()) };
    Ok(Declared {
      name: table.to_string(),
      create: create["sql"].as_str().unwrap_or_default().to_string(),
      columns: names("SELECT name FROM pragma_table_xinfo(?) WHERE hidden = 0 ORDER BY cid")?,
      key: names("SELECT name FROM pragma_table_xinfo(?) WHERE pk > 0 ORDER BY pk")?,
    })
  }

  // The row a key names, as a connection to the store holds it now, its
  // values as SQLite holds them.
  pub(crate) fn row(&self, connection: &Connection, failure: &Failure, row: &Row) -> Result<Option<Values>> {
    let key: Vec<Value> = serde_json::from_str(&row.key)?;
    let matching = self.key.iter().map(|column| format!("{} = ?", quoted(column))).collect::<Vec<_>>().join(" AND ");
    let sql = format!("SELECT {} FROM main.{} WHERE {matching}", self.listed(self.columns.len()), quoted(&self.name));
    let width = self.columns.len();
    let found = connection.prepare(&sql).and_then(|mut statement| statement.query_row(params_from_iter(key.iter().map(to_sql)), |values| (0..width).map(|column| values.get(column)).collect()).optional());
    checked(failure, found)
  }

  // The first columns, as a statement lists them.
  fn listed(&self, count: usize) -> String {
    self.columns.iter().take(count).map(|column| quoted(column)).collect::<Vec<_>>().join(", ")
  }
}

// An identifier as SQL quotes it.
fn quoted(name: &str) -> String {
  format!("\"{}\"", name.replace('"', "\"\""))
}

pub(crate) struct Scratch {
  connection: Connection,
  access: Shared,
  failure: Failure,
  // Each table declared here, by name, with the statement that declared it.
  declared: RefCell<HashMap<String, String>>,
}

impl Scratch {
  pub(crate) fn open(failure: Failure) -> Result<Scratch> {
    let connection = Connection::open_in_memory().map_err(sqlite)?;
    let access = guard(&connection)?;
    Ok(Scratch { connection, access, failure, declared: RefCell::default() })
  }

  // Hold one row of a table, alone, or none: declared as the store declares
  // it, it names only the columns it has values for, so a column added since
  // takes its default.
  pub(crate) fn hold(&self, table: &Declared, row: Option<&Values>) -> Result<()> {
    let name = quoted(&table.name);
    if self.declared.borrow().get(&table.name) != Some(&table.create) {
      batch(&self.connection, &self.failure, &format!("DROP TABLE IF EXISTS {name}; {}", table.create))?;
      self.declared.borrow_mut().insert(table.name.clone(), table.create.clone());
    }
    batch(&self.connection, &self.failure, &format!("DELETE FROM {name}"))?;
    let Some(values) = row else { return Ok(()) };
    let holes = vec!["?"; values.len()].join(", ");
    let insert = format!("INSERT INTO {name} ({}) VALUES ({holes})", table.listed(values.len()));
    checked(&self.failure, self.connection.execute(&insert, params_from_iter(values))).map(|_| ())
  }

  // Run a service's statement against the row held, inside its call window,
  // returning its rows and what it read: that row, when it read only it.
  pub(crate) fn read(&self, window: &CallWindow, looked_up: &Row, sql: &str, params: &[Value]) -> (Result<Vec<Value>>, Vec<Read>) {
    start_reading(&self.access);
    let rows = within(&self.access, window, Some(looked_up.clone()), || all(&self.connection, &self.failure, sql, params));
    (rows, stop_reading(&self.access))
  }
}
