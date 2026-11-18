//! One SQLite file, the store behind a space or the system database: its
//! connections, its transactions, the changeset each op's transaction records,
//! and the first system failure met on it.
//!
//! A store keeps one writer connection, on which one op's transaction runs at a
//! time. A session records the transaction's changes as a changeset: what a
//! rewind undoes, and the write set, the rows it changed by table and key;
//! watches re-run on their tables, since it holds only rows whose values
//! changed. The authorizer records the tables the
//! transaction's statements read, its read set. A call made inside an op runs under a
//! savepoint, so its writes roll back alone when it fails, and leave nothing in
//! the changeset. The op log, results and the core's migrations are left out of
//! it. The core's own tables move forward by migrations, applied when the file
//! opens and tracked in `carbon_migrations`.
//!
//! Each top-level GET opens its own read-only `Reader`, which in WAL mode sees
//! only committed state and records every table it reads, the read set a watch
//! re-runs on. Reads therefore never wait for a write, nor for each other.
//!
//! A service's statements run inside a call window
//! (`crate::sql::call_window`); the core's own run outside one. Statements
//! are prepared fresh each time, never from a
//! cache, because the authorizer only runs when a statement is prepared.
//!
//! No statement waits for a lock: another process holding the file is a system
//! failure, like an I/O error or corruption, and it is recorded here so the
//! space stops whatever the statement's caller does with the error.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use rusqlite::session::{ConflictAction, Session, invert_strm};
use rusqlite::types::Value as Sql;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Batch, Connection, OpenFlags, params_from_iter};
use self_cell::self_cell;
use serde_json::{Map, Value};

pub use carbondb_service_bindings::Db;
use carbondb_service_bindings::migrations::{self, Migration};

pub mod changeset;

use crate::sql::call_window::{CallWindow, Scoped, guard};
use crate::sql::Statements;
use crate::sql::authorizer::{Read, Shared, start_reading, stop_reading, unrecorded};
use crate::error::{Result, ServiceError, bad_request, is_system_failure, sqlite, system_failure};

// The core's own record of a space, which no rewind undoes.
const UNRECORDED: [&str; 3] = ["carbon_pointer", "carbon_result", MIGRATED];

// Where a file tracks the migrations of the core's own tables.
const MIGRATED: &str = "carbon_migrations";

// The first system failure met on a store's connections, shared with its readers.
pub(crate) type Failure = Rc<RefCell<Option<ServiceError>>>;

type Recording<'a> = RefCell<Option<Session<'a>>>;

self_cell!(
  // The writer connection, and the session recording the open write transaction.
  struct Writer {
    owner: Connection,
    #[not_covariant]
    dependent: Recording,
  }
);

pub struct Store {
  path: PathBuf,
  writer: Writer,
  access: Shared,
  failure: Failure,
  statements: Rc<Statements>,
}

impl Store {
  pub fn open(path: &Path) -> Result<Store> {
    let connection = Connection::open(path).map_err(sqlite)?;
    connection.busy_timeout(Duration::ZERO).map_err(sqlite)?;
    connection.pragma_update(None, "journal_mode", "wal").map_err(sqlite)?;
    connection.pragma_update(None, "foreign_keys", "on").map_err(sqlite)?;
    let access = guard(&connection)?;
    Ok(Store { path: path.to_path_buf(), writer: Writer::new(connection, |_| RefCell::default()), access, failure: Failure::default(), statements: Rc::default() })
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  fn connection(&self) -> &Connection {
    self.writer.borrow_owner()
  }

  // The system failure met on this store, if any: the space must stop.
  pub fn failure(&self) -> Option<ServiceError> {
    self.failure.borrow().clone()
  }

  // Forget the failure once it is fixed by hand, as the space resumes.
  pub fn clear_failure(&self) {
    self.failure.borrow_mut().take();
  }

  // Open a write transaction and start recording its changes and the tables it reads.
  pub fn begin_write(&self) -> Result<()> {
    batch(self.connection(), &self.failure, "BEGIN IMMEDIATE")?;
    start_reading(&self.access);
    self.writer.with_dependent(|connection, recording| {
      let mut session = Session::new(connection).map_err(sqlite)?;
      session.table_filter(Some(|table: &str| !UNRECORDED.contains(&table)));
      session.attach(None::<&str>).map_err(sqlite)?;
      *recording.borrow_mut() = Some(session);
      Ok(())
    })
  }

  // End the write transaction, committing or rolling back, and stop recording
  // it. Its changeset is read before, with `changeset`.
  pub fn end_write(&self, commit: bool) -> Result<()> {
    stop_reading(&self.access);
    self.writer.with_dependent(|_, recording| recording.borrow_mut().take());
    batch(self.connection(), &self.failure, if commit { "COMMIT" } else { "ROLLBACK" })
  }

  // The open transaction's read set so far: the tables and rows its
  // statements read, leaving out the core's own record.
  pub fn read_set(&self) -> Vec<Read> {
    let mut access = self.access.lock().expect("access");
    access.read().iter().filter(|read| !UNRECORDED.contains(&read.table())).cloned().collect()
  }

  // The open transaction's changeset so far: the one place a session's
  // changes become bytes. The session reads the rows it changed to build it,
  // which is no read of the op's.
  pub fn changeset(&self) -> Result<Vec<u8>> {
    unrecorded(&self.access, || {
      self.writer.with_dependent(|_, recording| match recording.borrow_mut().as_mut() {
        Some(session) => {
          let mut bytes = vec![];
          session.changeset_strm(&mut bytes).map_err(sqlite).map(|_| bytes)
        }
        None => Ok(vec![]),
      })
    })
  }

  // Undo a committed changeset inside the open transaction by applying its
  // inverse. Undoing changesets newest first, back to an op, leaves the rows
  // they wrote as they stood before it, so the inverse never meets a row that
  // differs from what it expects; when it does, the space no longer matches
  // its own record, a system failure.
  pub fn undo(&self, changeset: &[u8]) -> Result<()> {
    let mut inverse = vec![];
    checked(&self.failure, invert_strm(&mut &changeset[..], &mut inverse))?;
    let input: &mut dyn std::io::Read = &mut &inverse[..];
    let applied = self.connection().apply_strm(input, None::<fn(&str) -> bool>, |_, _| ConflictAction::SQLITE_CHANGESET_ABORT);
    applied.map_err(|error| {
      let failure = system_failure(format!("Undoing a changeset met rows it did not write: {error}"));
      self.failure.borrow_mut().get_or_insert_with(|| failure.clone());
      failure
    })
  }

  // A call inside the open transaction starts here; its writes keep or roll back on their own.
  // Savepoints share one name: the innermost is always the one released or rolled back to.
  pub fn savepoint(&self) -> Result<()> {
    batch(self.connection(), &self.failure, "SAVEPOINT carbon_call")
  }

  // End the innermost savepoint, keeping its writes or undoing them.
  pub fn end_savepoint(&self, keep: bool) -> Result<()> {
    batch(self.connection(), &self.failure, if keep { "RELEASE carbon_call" } else { "ROLLBACK TO carbon_call; RELEASE carbon_call" })
  }

  // Run a synchronous write in one transaction: commit on success, roll back
  // on error. Returns its value and changeset.
  pub fn write<T>(&self, work: impl FnOnce(&Store) -> Result<T>) -> Result<(T, Vec<u8>)> {
    self.begin_write()?;
    let done = work(self).and_then(|value| Ok((value, self.changeset()?)));
    match done {
      Ok(done) => self.end_write(true).map(|_| done),
      Err(error) => {
        let _ = self.end_write(false);
        Err(error)
      }
    }
  }

  // A statement of the core's own whose parameters need SQLite's own types,
  // such as a changeset's bytes, which the JSON `Db` binding cannot carry.
  pub(crate) fn run_typed(&self, sql: &str, params: &[Sql]) -> Result<u64> {
    checked(&self.failure, self.connection().execute(sql, params_from_iter(params))).map(|changed| changed as u64)
  }

  // A read on its own connection, over a snapshot of committed state; a
  // system failure it meets stops this store's space.
  pub fn reader(&self) -> Result<Reader> {
    Reader::over(&self.path, Rc::clone(&self.failure), Rc::clone(&self.statements))
  }

  // Bring the core's own tables in this file up to date, in one transaction:
  // forward only, refusing a file that newer code has migrated.
  pub fn migrate(&self, migrations: &[Migration]) -> Result<()> {
    self.write(|store| migrations::apply(store, MIGRATED, migrations)).map(|_| ())
  }

  // Several statements with no parameters.
  pub fn run_batch(&self, sql: &str) -> Result<()> {
    batch(self.connection(), &self.failure, sql)
  }

  // The writer inside a service's call window.
  pub fn scoped(&self, window: CallWindow) -> Scoped<'_> {
    Scoped { connection: self.connection(), access: &self.access, failure: &self.failure, statements: &self.statements, window }
  }

}

impl Db for Store {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    run(self.connection(), &self.failure, sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    all(self.connection(), &self.failure, sql, params)
  }
}

// A GET's snapshot of committed state, recording the tables it reads.
pub struct Reader {
  connection: Connection,
  access: Shared,
  failure: Failure,
  statements: Rc<Statements>,
}

impl Reader {
  // A read of a file no store holds open here, over a snapshot of committed state.
  pub fn open(path: &Path) -> Result<Reader> {
    Reader::over(path, Failure::default(), Rc::default())
  }

  fn over(path: &Path, failure: Failure, statements: Rc<Statements>) -> Result<Reader> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sqlite)?;
    connection.busy_timeout(Duration::ZERO).map_err(sqlite)?;
    let access = guard(&connection)?;
    batch(&connection, &failure, "BEGIN")?;
    start_reading(&access);
    Ok(Reader { connection, access, failure, statements })
  }

  // End the read and return the tables it read, which watches re-run on.
  pub fn end(&self) -> Result<Vec<String>> {
    let ended = batch(&self.connection, &self.failure, "COMMIT");
    let tables: std::collections::BTreeSet<String> = stop_reading(&self.access).iter().map(|read| read.table().to_string()).collect();
    ended.map(|_| tables.into_iter().collect())
  }

  pub fn scoped(&self, window: CallWindow) -> Scoped<'_> {
    Scoped { connection: &self.connection, access: &self.access, failure: &self.failure, statements: &self.statements, window }
  }
}

impl Db for Reader {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    run(&self.connection, &self.failure, sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    all(&self.connection, &self.failure, sql, params)
  }
}

// A SQLite result as the core's, noting a system failure on the store.
pub(crate) fn checked<T>(failure: &Failure, result: rusqlite::Result<T>) -> Result<T> {
  result.map_err(|error| {
    let system = is_system_failure(&error);
    let error = sqlite(error);
    if system {
      failure.borrow_mut().get_or_insert_with(|| error.clone());
    }
    error
  })
}

pub(crate) fn batch(connection: &Connection, failure: &Failure, sql: &str) -> Result<()> {
  checked(failure, connection.execute_batch(sql))
}

// Run SQL as the `db` binding promises: one statement with its parameters, or
// several with none, as a migration holds. Returns the rows it changed.
pub(crate) fn run(connection: &Connection, failure: &Failure, sql: &str, params: &[Value]) -> Result<u64> {
  match params.is_empty() {
    true => each(connection, failure, sql),
    false => one(connection, failure, sql, params),
  }
}

// One statement with its parameters; several are refused before any runs.
fn one(connection: &Connection, failure: &Failure, sql: &str, params: &[Value]) -> Result<u64> {
  match connection.execute(sql, params_from_iter(params.iter().map(to_sql))) {
    Err(rusqlite::Error::MultipleStatement) => Err(bad_request("Several statements run only without parameters")),
    executed => checked(failure, executed).map(|changed| changed as u64),
  }
}

// Statements with no parameters, each prepared once the one before it has run,
// since it may name what that one created.
fn each(connection: &Connection, failure: &Failure, sql: &str) -> Result<u64> {
  let mut statements = Batch::new(connection, sql);
  let mut changed = None;
  while let Some(mut statement) = checked(failure, statements.next())? {
    changed = Some(changed.unwrap_or(0) + checked(failure, statement.execute([]))? as u64);
  }
  changed.ok_or_else(|| bad_request("No SQL statement to run"))
}

pub(crate) fn all(connection: &Connection, failure: &Failure, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
  let mut statement = checked(failure, connection.prepare(sql))?;
  let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
  let rows = statement.query_map(params_from_iter(params.iter().map(to_sql)), |row| {
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
      object.insert(column.clone(), from_sql(row.get(index)?));
    }
    Ok(Value::Object(object))
  });
  checked(failure, rows.and_then(|rows| rows.collect::<rusqlite::Result<Vec<Value>>>()))
}

// A JSON value as a SQLite value: strings and numbers bind directly, and only
// objects and arrays are stored as JSON text.
pub(crate) fn to_sql(value: &Value) -> Sql {
  match value {
    Value::Null => Sql::Null,
    Value::Bool(flag) => Sql::Integer(*flag as i64),
    Value::Number(number) => number.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(number.as_f64().unwrap_or_default())),
    Value::String(text) => Sql::Text(text.clone()),
    nested => Sql::Text(nested.to_string()),
  }
}

pub(crate) fn from_sql(value: Sql) -> Value {
  match value {
    Sql::Null => Value::Null,
    Sql::Integer(number) => Value::from(number),
    Sql::Real(number) => Value::from(number),
    Sql::Text(text) => Value::from(text),
    Sql::Blob(bytes) => Value::from(bytes),
  }
}

// Hold a store's file from another connection, as another process would.
#[cfg(test)]
pub(crate) fn hold_lock(path: &Path) -> Connection {
  let holder = Connection::open(path).unwrap();
  holder.execute_batch("BEGIN IMMEDIATE").unwrap();
  holder
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::store::changeset::{rows, tables};
  use serde_json::json;

  fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("space.db")).unwrap();
    store
      .run_batch("CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID; CREATE TABLE member (id TEXT PRIMARY KEY) WITHOUT ROWID; CREATE TABLE carbon_result (hash TEXT PRIMARY KEY) WITHOUT ROWID")
      .unwrap();
    (directory, store)
  }

  fn texts(db: &dyn Db) -> Vec<Value> {
    db.all("SELECT id FROM message ORDER BY id", &[]).unwrap().into_iter().map(|row| row["id"].clone()).collect()
  }

  // A case of `run`: its SQL and parameters, the rows it changed or the status
  // it failed with, and the messages left after it.
  type RunCase = (&'static str, &'static str, Vec<Value>, std::result::Result<u64, u16>, Vec<Value>);

  #[test]
  fn runs_one_statement_with_its_parameters_or_several_without_each_in_turn() {
    let cases: [RunCase; 5] = [
      ("one statement with its parameters", "INSERT INTO message (id) VALUES (?)", vec![json!("a")], Ok(1), vec![json!("a")]),
      (
        "several without, each prepared once the one before it has run",
        "CREATE TABLE extra (id TEXT PRIMARY KEY) WITHOUT ROWID; INSERT INTO extra (id) VALUES ('x'); INSERT INTO message (id) VALUES ('b'), ('c')",
        vec![],
        Ok(3),
        vec![json!("b"), json!("c")],
      ),
      ("several with parameters, refused before any runs", "INSERT INTO message (id) VALUES (?); INSERT INTO message (id) VALUES ('d')", vec![json!("a")], Err(400), vec![]),
      ("no statement", "", vec![], Err(400), vec![]),
      ("only a comment", "-- nothing to run", vec![], Err(400), vec![]),
    ];
    for (name, sql, params, expected, ids) in cases {
      let (_directory, store) = store();
      assert_eq!(store.run(sql, &params).map_err(|error| error.status), expected, "{name}");
      assert_eq!(texts(&store), ids, "{name}");
    }
  }

  #[test]
  fn a_write_reports_the_tables_whose_rows_it_changed_leaving_out_the_cores_own() {
    let (_directory, store) = store();
    store.run("INSERT INTO member (id) VALUES ('same')", &[]).unwrap();
    let (_, changeset) = store
      .write(|store| {
        store.run("INSERT INTO message (id, text) VALUES (?, ?)", &[json!("m1"), json!("hi")])?;
        store.run("UPDATE member SET id = 'same' WHERE id = 'same'", &[])?;
        store.run("DELETE FROM member WHERE id = 'nobody'", &[])?;
        store.run("INSERT INTO carbon_result (hash) VALUES ('h')", &[])?;
        store.all("SELECT * FROM member", &[])
      })
      .unwrap();
    assert_eq!(tables(&changeset).unwrap(), vec!["message"]);
  }

  #[test]
  fn a_write_reports_each_row_it_changed_by_its_table_and_key() {
    let (_directory, store) = store();
    store.run("INSERT INTO message (id, text) VALUES ('m2', 'old'), ('m3', 'gone')", &[]).unwrap();
    let (_, changeset) = store
      .write(|store| {
        store.run("INSERT INTO message (id, text) VALUES ('m1', 'new')", &[])?;
        store.run("UPDATE message SET text = 'edited' WHERE id = 'm2'", &[])?;
        store.run("DELETE FROM message WHERE id = 'm3'", &[])
      })
      .unwrap();
    let rows: Vec<(String, String)> = rows(&changeset).unwrap().into_iter().map(|row| (row.table, row.key)).collect();
    let expected = [("message", r#"["m1"]"#), ("message", r#"["m2"]"#), ("message", r#"["m3"]"#)];
    assert_eq!(rows, expected.map(|(table, key)| (table.to_string(), key.to_string())));
  }

  #[test]
  fn a_savepoint_rolled_back_leaves_nothing_in_the_changeset() {
    let (_directory, store) = store();
    store.begin_write().unwrap();
    store.run("INSERT INTO message (id) VALUES ('kept')", &[]).unwrap();
    store.savepoint().unwrap();
    store.run("INSERT INTO member (id) VALUES ('undone')", &[]).unwrap();
    store.end_savepoint(false).unwrap();
    assert_eq!(tables(&store.changeset().unwrap()).unwrap(), vec!["message"]);
    store.end_write(true).unwrap();
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
    assert_eq!(store.failure(), None);
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

  // Another process holding the file fails at once, and the store keeps the
  // failure whichever statement met it, a service's included.
  #[test]
  fn records_the_first_system_failure_met_on_any_statement() {
    let (directory, store) = store();
    let holder = hold_lock(&directory.path().join("space.db"));
    let window = CallWindow { service: true, op: None };
    let error = store.scoped(window).run("CREATE TABLE note (id TEXT PRIMARY KEY) WITHOUT ROWID", &[]).unwrap_err();
    assert_eq!((error.status, error.code.as_str()), (503, "SYSTEM_FAILURE"));
    assert_eq!(store.begin_write().unwrap_err().status, 503);
    drop(holder);
    assert_eq!(store.failure(), Some(error));
    assert!(store.run("INSERT INTO message (id) VALUES ('m1')", &[]).is_ok());
    assert!(store.failure().is_some_and(|failure| failure.status == 503));
  }
}
