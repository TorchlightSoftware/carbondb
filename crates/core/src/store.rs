//! One SQLite file, the store behind a space or the system database.
//!
//! A store keeps one writer connection, on which one op's transaction runs at a
//! time; while it is open, the authorizer records every table it writes, the
//! write set a commit notice carries. A call made inside an op runs under a
//! savepoint, so its writes roll back alone when it fails.
//!
//! Each top-level GET opens its own read-only `Reader`, which in WAL mode sees
//! only committed state and records every table it reads, the read set `watch`
//! re-runs on. Reads therefore never wait for a write, nor for each other.
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

pub use carbondb_service_api::Db;

use crate::error::{Result, sqlite};

// The tables a transaction or read has touched. SQLite's authorizer must be Send.
type TableSet = Arc<Mutex<BTreeSet<String>>>;

// Which way a connection's touched tables are recorded: writes on the writer, reads on a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touch {
  Write,
  Read,
}

pub struct Store {
  path: PathBuf,
  writer: Connection,
  written: TableSet,
}

impl Store {
  pub fn open(path: &Path) -> Result<Store> {
    let writer = Connection::open(path).map_err(sqlite)?;
    writer.pragma_update(None, "journal_mode", "wal").map_err(sqlite)?;
    writer.pragma_update(None, "foreign_keys", "on").map_err(sqlite)?;
    Ok(Store { path: path.to_path_buf(), writer, written: TableSet::default() })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  // Open a write transaction and start recording the tables it writes.
  pub fn begin_write(&self) -> Result<()> {
    self.writer.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
    record(&self.writer, Touch::Write, &self.written)
  }

  // End the write transaction, committing or rolling back, and return the tables it wrote.
  pub fn end_write(&self, commit: bool) -> Result<Vec<String>> {
    let ended = self.writer.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" });
    let tables = stop_recording(&self.writer, &self.written)?;
    ended.map_err(sqlite)?;
    Ok(tables)
  }

  // A call inside the open transaction starts here; its writes keep or roll back on their own.
  // Savepoints share one name: the innermost is always the one released or rolled back to.
  pub fn savepoint(&self) -> Result<()> {
    self.writer.execute_batch("SAVEPOINT carbon_call").map_err(sqlite)
  }

  // End the innermost savepoint, keeping its writes or undoing them.
  pub fn end_savepoint(&self, keep: bool) -> Result<()> {
    let sql = if keep { "RELEASE carbon_call" } else { "ROLLBACK TO carbon_call; RELEASE carbon_call" };
    self.writer.execute_batch(sql).map_err(sqlite)
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

  // A read on its own connection, over a snapshot of committed state.
  pub fn reader(&self) -> Result<Reader> {
    let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sqlite)?;
    connection.execute_batch("BEGIN").map_err(sqlite)?;
    let read = TableSet::default();
    record(&connection, Touch::Read, &read)?;
    Ok(Reader { connection, read })
  }

  // Several statements with no parameters, such as a migration.
  pub fn run_batch(&self, sql: &str) -> Result<()> {
    self.writer.execute_batch(sql).map_err(sqlite)
  }
}

impl Db for Store {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    run(&self.writer, sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    all(&self.writer, sql, params)
  }
}

// A GET's snapshot of committed state, recording the tables it reads.
pub struct Reader {
  connection: Connection,
  read: TableSet,
}

impl Reader {
  // End the read and return the tables it read.
  pub fn end(&self) -> Result<Vec<String>> {
    let ended = self.connection.execute_batch("COMMIT");
    let tables = stop_recording(&self.connection, &self.read)?;
    ended.map_err(sqlite)?;
    Ok(tables)
  }
}

impl Db for Reader {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    run(&self.connection, sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    all(&self.connection, sql, params)
  }
}

// Record the tables a connection writes, or reads, until recording stops.
fn record(connection: &Connection, touch: Touch, tables: &TableSet) -> Result<()> {
  tables.lock().expect("table set").clear();
  let touched = Arc::clone(tables);
  connection.authorizer(Some(move |context: AuthContext<'_>| {
    if let Some(table) = touched_table(touch, &context.action) {
      touched.lock().expect("table set").insert(table.to_string());
    }
    Authorization::Allow
  })).map_err(sqlite)
}

fn stop_recording(connection: &Connection, tables: &TableSet) -> Result<Vec<String>> {
  connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>).map_err(sqlite)?;
  Ok(std::mem::take(&mut *tables.lock().expect("table set")).into_iter().collect())
}

fn run(connection: &Connection, sql: &str, params: &[Value]) -> Result<u64> {
  Ok(connection.execute(sql, params_from_iter(params.iter().map(to_sql))).map_err(sqlite)? as u64)
}

fn all(connection: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
  let mut statement = connection.prepare(sql).map_err(sqlite)?;
  let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
  let rows = statement.query_map(params_from_iter(params.iter().map(to_sql)), |row| {
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
      object.insert(column.clone(), from_sql(row.get(index)?));
    }
    Ok(Value::Object(object))
  }).map_err(sqlite)?;
  rows.collect::<rusqlite::Result<Vec<Value>>>().map_err(sqlite)
}

// The table an authorizer action writes, on the writer, or reads, on a reader.
// SQLite's own tables are left out.
fn touched_table<'a>(touch: Touch, action: &AuthAction<'a>) -> Option<&'a str> {
  let table = match (touch, action) {
    (Touch::Write, AuthAction::Insert { table_name } | AuthAction::Delete { table_name } | AuthAction::Update { table_name, .. }) => table_name,
    (Touch::Read, AuthAction::Read { table_name, .. }) => table_name,
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

  fn texts(db: &dyn Db) -> Vec<Value> {
    db.all("SELECT id FROM message ORDER BY id", &[]).unwrap().into_iter().map(|row| row["id"].clone()).collect()
  }

  #[test]
  fn a_write_reports_the_tables_it_wrote() {
    let (_directory, store) = store();
    let (_, tables) = store
      .write(|store| {
        store.run("INSERT INTO message (id, text) VALUES (?, ?)", &[json!("m1"), json!("hi")])?;
        store.all("SELECT * FROM member", &[])
      })
      .unwrap();
    assert_eq!(tables, vec!["message"]);
  }

  #[test]
  fn a_read_reports_the_tables_it_read_and_sees_only_committed_state() {
    let (_directory, store) = store();
    store.begin_write().unwrap();
    store.run("INSERT INTO message (id, text) VALUES ('m1', 'hi')", &[]).unwrap();
    let reader = store.reader().unwrap();
    let rows = reader.all("SELECT m.id FROM message m JOIN member p ON p.id = m.id", &[]).unwrap();
    assert_eq!((rows, reader.end().unwrap()), (vec![], vec!["member".to_string(), "message".to_string()]));
    store.end_write(true).unwrap();
    assert_eq!(store.reader().unwrap().get("SELECT text FROM message", &[]).unwrap(), json!({"text": "hi"}));
  }

  #[test]
  fn a_failed_write_rolls_back() {
    let (_directory, store) = store();
    let result = store.write(|store| {
      store.run("INSERT INTO message (id) VALUES ('m1')", &[])?;
      store.run("INSERT INTO message (id) VALUES ('m1')", &[])
    });
    assert_eq!(result.unwrap_err().status, 500);
    assert_eq!(texts(&store.reader().unwrap()), Vec::<Value>::new());
  }

  #[test]
  fn a_savepoint_rolls_back_only_its_own_writes_at_any_depth() {
    let (_directory, store) = store();
    store.begin_write().unwrap();
    store.run("INSERT INTO message (id) VALUES ('outer')", &[]).unwrap();
    store.savepoint().unwrap();
    store.run("INSERT INTO message (id) VALUES ('kept')", &[]).unwrap();
    store.savepoint().unwrap();
    store.run("INSERT INTO message (id) VALUES ('undone')", &[]).unwrap();
    store.end_savepoint(false).unwrap();
    store.end_savepoint(true).unwrap();
    store.end_write(true).unwrap();
    assert_eq!(texts(&store.reader().unwrap()), vec![json!("kept"), json!("outer")]);
  }

  #[test]
  fn a_reader_cannot_write() {
    let (_directory, store) = store();
    assert!(store.reader().unwrap().run("INSERT INTO message (id) VALUES ('m1')", &[]).unwrap_err().message.contains("readonly"));
  }

  #[test]
  fn json_values_round_trip_through_sqlite() {
    let (_directory, store) = store();
    // An untyped column keeps each value's own type; a TEXT column would coerce numbers.
    store.run_batch("CREATE TABLE cell (id TEXT PRIMARY KEY, value)").unwrap();
    let cases = [json!("hello"), json!(42), json!(1.5), json!(null), json!({"nested": [1]})];
    for (index, value) in cases.iter().enumerate() {
      store.run("INSERT INTO cell (id, value) VALUES (?, ?)", &[json!(index.to_string()), value.clone()]).unwrap();
    }
    let values: Vec<Value> = store.reader().unwrap().all("SELECT value FROM cell ORDER BY id", &[]).unwrap().into_iter().map(|row| row["value"].clone()).collect();
    assert_eq!(values, vec![json!("hello"), json!(42), json!(1.5), json!(null), json!("{\"nested\":[1]}")]);
  }
}
