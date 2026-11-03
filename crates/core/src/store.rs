//! One SQLite file, the store behind a space or the system database.
//!
//! A store has two connections. The writer applies one transaction at a time;
//! while one is open, the authorizer records every table it writes, the write
//! set a commit notice carries. The reader is read-only and, in WAL mode, sees
//! only committed state; while a read is open, the authorizer records every
//! table it reads, the read set `watch` re-runs on.
//!
//! Statements are prepared fresh each time, never from a cache, because the
//! authorizer only runs when a statement is prepared.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde_json::{Map, Value};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
  Writer,
  Reader,
}

// The tables an open transaction or read has touched.
type TableSet = Arc<Mutex<BTreeSet<String>>>;

pub struct Store {
  path: PathBuf,
  writer: Connection,
  reader: Connection,
  // Each side records separately, since a read may run while a write is open.
  written: TableSet,
  read: TableSet,
}

impl Store {
  pub fn open(path: &Path) -> Result<Store> {
    let writer = Connection::open(path)?;
    writer.pragma_update(None, "journal_mode", "wal")?;
    writer.pragma_update(None, "foreign_keys", "on")?;
    let reader = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    Ok(Store { path: path.to_path_buf(), writer, reader, written: TableSet::default(), read: TableSet::default() })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  fn connection(&self, side: Side) -> &Connection {
    match side {
      Side::Writer => &self.writer,
      Side::Reader => &self.reader,
    }
  }

  fn tables(&self, side: Side) -> &TableSet {
    match side {
      Side::Writer => &self.written,
      Side::Reader => &self.read,
    }
  }

  // Open a write transaction and start recording the tables it writes.
  pub fn begin_write(&self) -> Result<()> {
    self.writer.execute_batch("BEGIN IMMEDIATE")?;
    self.record(Side::Writer)
  }

  // End the write transaction, committing or rolling back, and return the tables it wrote.
  pub fn end_write(&self, commit: bool) -> Result<Vec<String>> {
    let ended = self.writer.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" });
    let tables = self.stop_recording(Side::Writer)?;
    ended?;
    Ok(tables)
  }

  // Open a read on a snapshot of committed state and start recording the tables it reads.
  pub fn begin_read(&self) -> Result<()> {
    self.reader.execute_batch("BEGIN")?;
    self.record(Side::Reader)
  }

  pub fn end_read(&self) -> Result<Vec<String>> {
    let ended = self.reader.execute_batch("COMMIT");
    let tables = self.stop_recording(Side::Reader)?;
    ended?;
    Ok(tables)
  }

  // Run a synchronous write in one transaction: commit on success, roll back on error.
  pub fn write<T>(&self, work: impl FnOnce(&Store) -> Result<T>) -> Result<(T, Vec<String>)> {
    self.begin_write()?;
    match work(self) {
      Ok(value) => Ok((value, self.end_write(true)?)),
      Err(error) => {
        let _ = self.end_write(false);
        Err(error)
      }
    }
  }

  pub fn read<T>(&self, work: impl FnOnce(&Store) -> Result<T>) -> Result<(T, Vec<String>)> {
    self.begin_read()?;
    let value = work(self);
    let tables = self.end_read()?;
    Ok((value?, tables))
  }

  // Record the tables the writer writes, or the reader reads, until recording stops.
  fn record(&self, side: Side) -> Result<()> {
    let touched = Arc::clone(self.tables(side));
    touched.lock().expect("table set").clear();
    self.connection(side).authorizer(Some(move |context: AuthContext<'_>| {
      if let Some(table) = touched_table(side, &context.action) {
        touched.lock().expect("table set").insert(table.to_string());
      }
      Authorization::Allow
    }))?;
    Ok(())
  }

  fn stop_recording(&self, side: Side) -> Result<Vec<String>> {
    self.connection(side).authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    let tables = std::mem::take(&mut *self.tables(side).lock().expect("table set"));
    Ok(tables.into_iter().collect())
  }

  pub fn run(&self, side: Side, sql: &str, params: &[Value]) -> Result<u64> {
    Ok(self.connection(side).execute(sql, params_from_iter(params.iter().map(to_sql)))? as u64)
  }

  // Several statements with no parameters, such as a migration.
  pub fn run_batch(&self, sql: &str) -> Result<()> {
    Ok(self.writer.execute_batch(sql)?)
  }

  pub fn all(&self, side: Side, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    let connection = self.connection(side);
    let mut statement = connection.prepare(sql)?;
    let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
    let rows = statement.query_map(params_from_iter(params.iter().map(to_sql)), |row| {
      let mut object = Map::new();
      for (index, column) in columns.iter().enumerate() {
        object.insert(column.clone(), from_sql(row.get(index)?));
      }
      Ok(Value::Object(object))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<Value>>>()?)
  }

  // The first row, or null when there is none.
  pub fn get(&self, side: Side, sql: &str, params: &[Value]) -> Result<Value> {
    Ok(self.all(side, sql, params)?.into_iter().next().unwrap_or(Value::Null))
  }
}

// The table an authorizer action writes, on the writer, or reads, on the reader.
// SQLite's own tables are left out.
fn touched_table<'a>(side: Side, action: &AuthAction<'a>) -> Option<&'a str> {
  let table = match (side, action) {
    (Side::Writer, AuthAction::Insert { table_name } | AuthAction::Delete { table_name } | AuthAction::Update { table_name, .. }) => table_name,
    (Side::Reader, AuthAction::Read { table_name, .. }) => table_name,
    _ => return None,
  };
  (!table.starts_with("sqlite_")).then_some(table)
}

// A JSON value as a SQLite value: strings and numbers bind directly, and only
// objects and arrays are stored as JSON text.
fn to_sql(value: &Value) -> Sql {
  match value {
    Value::Null => Sql::Null,
    Value::Bool(flag) => Sql::Integer(*flag as i64),
    Value::Number(number) => number.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(number.as_f64().unwrap_or_default())),
    Value::String(text) => Sql::Text(text.clone()),
    nested => Sql::Text(nested.to_string()),
  }
}

fn from_sql(value: Sql) -> Value {
  match value {
    Sql::Null => Value::Null,
    Sql::Integer(number) => Value::from(number),
    Sql::Real(number) => Value::from(number),
    Sql::Text(text) => Value::from(text),
    Sql::Blob(bytes) => Value::from(bytes),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("space.db")).unwrap();
    store.run_batch("CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT); CREATE TABLE member (id TEXT PRIMARY KEY)").unwrap();
    (directory, store)
  }

  #[test]
  fn a_write_reports_the_tables_it_wrote() {
    let (_directory, store) = store();
    let (_, tables) = store
      .write(|store| {
        store.run(Side::Writer, "INSERT INTO message (id, text) VALUES (?, ?)", &[json!("m1"), json!("hi")])?;
        store.all(Side::Writer, "SELECT * FROM member", &[])
      })
      .unwrap();
    assert_eq!(tables, vec!["message"]);
  }

  #[test]
  fn a_read_reports_the_tables_it_read_and_sees_only_committed_state() {
    let (_directory, store) = store();
    store.begin_write().unwrap();
    store.run(Side::Writer, "INSERT INTO message (id, text) VALUES ('m1', 'hi')", &[]).unwrap();
    let (rows, tables) = store.read(|store| store.all(Side::Reader, "SELECT m.id FROM message m JOIN member p ON p.id = m.id", &[])).unwrap();
    assert_eq!((rows, tables), (vec![], vec!["member".to_string(), "message".to_string()]));
    store.end_write(true).unwrap();
    assert_eq!(store.get(Side::Reader, "SELECT text FROM message", &[]).unwrap(), json!({"text": "hi"}));
  }

  #[test]
  fn a_failed_write_rolls_back() {
    let (_directory, store) = store();
    let result = store.write(|store| {
      store.run(Side::Writer, "INSERT INTO message (id) VALUES ('m1')", &[])?;
      store.run(Side::Writer, "INSERT INTO message (id) VALUES ('m1')", &[])
    });
    assert_eq!(result.unwrap_err().status, 500);
    assert_eq!(store.all(Side::Reader, "SELECT * FROM message", &[]).unwrap(), Vec::<Value>::new());
  }

  #[test]
  fn the_reader_cannot_write() {
    let (_directory, store) = store();
    assert!(store.run(Side::Reader, "INSERT INTO message (id) VALUES ('m1')", &[]).unwrap_err().message.contains("readonly"));
  }

  #[test]
  fn json_values_round_trip_through_sqlite() {
    let (_directory, store) = store();
    // An untyped column keeps each value's own type; a TEXT column would coerce numbers.
    store.run_batch("CREATE TABLE cell (id TEXT PRIMARY KEY, value)").unwrap();
    let cases = [json!("hello"), json!(42), json!(1.5), json!(null), json!({"nested": [1]})];
    for (index, value) in cases.iter().enumerate() {
      store.run(Side::Writer, "INSERT INTO cell (id, value) VALUES (?, ?)", &[json!(index.to_string()), value.clone()]).unwrap();
    }
    let texts: Vec<Value> = store.all(Side::Reader, "SELECT value FROM cell ORDER BY id", &[]).unwrap().into_iter().map(|row| row["value"].clone()).collect();
    assert_eq!(texts, vec![json!("hello"), json!(42), json!(1.5), json!(null), json!("{\"nested\":[1]}")]);
  }
}
