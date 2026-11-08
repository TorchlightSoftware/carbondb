//! A call window: for the length of one statement, the service namespace and
//! op it runs within, installed on the connection and then restored.
//!
//! A service's statements run through `Scoped`, inside its call window: each
//! statement is rewritten into the service's namespace, SQLite's authorizer
//! refuses anything outside it, and `op_id()` draws from the window's op. The
//! core's own statements run outside any window, with neither.
//!
//! The authorizer also records what a GET's statements read, the read set
//! `watch` re-runs on. What an op writes comes from its changeset, in `store`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use serde_json::Value;

use crate::error::{Result, sqlite};
use crate::namespace::rewrite;
use crate::store::{Failure, all, run};
use crate::{Db, Op};

// The service namespace and op a statement runs within. The core's own
// statements have neither.
#[derive(Debug, Clone, Default)]
pub struct CallWindow {
  pub prefix: Option<String>,
  pub op: Option<Op>,
}

// What the statements on one connection may touch, and what they have read.
// SQLite's authorizer and functions must be Send, so it sits behind a mutex.
#[derive(Default)]
pub(crate) struct Access {
  window: CallWindow,
  reading: bool,
  read: BTreeSet<String>,
}

pub(crate) type Shared = Arc<Mutex<Access>>;

// Install the authorizer and `op_id()` on a connection, sharing one record of
// what its statements may touch and have read.
pub(crate) fn guard(connection: &Connection) -> Result<Shared> {
  let access = Shared::default();
  let authorizing = Arc::clone(&access);
  connection
    .authorizer(Some(move |context: AuthContext<'_>| authorize(&mut authorizing.lock().expect("access"), &context.action)))
    .map_err(sqlite)?;
  let drawing = Arc::clone(&access);
  connection
    .create_scalar_function("op_id", 0, FunctionFlags::SQLITE_UTF8, move |_| {
      let op = drawing.lock().expect("access").window.op.clone();
      op.map(|op| op_id(&op)).ok_or_else(|| rusqlite::Error::UserFunctionError("op_id() draws a key only inside an op".into()))
    })
    .map_err(sqlite)?;
  Ok(access)
}

// An integer key drawn from an op, below 2^53 so JavaScript reads it exactly.
pub fn op_id(op: &Op) -> i64 {
  (op.random() * (1u64 << 52) as f64) as i64
}

pub(crate) fn start_reading(access: &Shared) {
  let mut access = access.lock().expect("access");
  access.reading = true;
  access.read.clear();
}

// Stop recording and return the tables read.
pub(crate) fn stop_reading(access: &Shared) -> Vec<String> {
  let mut access = access.lock().expect("access");
  access.reading = false;
  std::mem::take(&mut access.read).into_iter().collect()
}

// Record what a statement reads, and refuse a service's statement that reaches
// outside its namespace.
fn authorize(access: &mut Access, action: &AuthAction<'_>) -> Authorization {
  if let Some(table) = read_table(action).filter(|_| access.reading) {
    access.read.insert(table.to_string());
  }
  match &access.window.prefix {
    Some(prefix) if !within(prefix, action) => Authorization::Deny,
    _ => Authorization::Allow,
  }
}

// The table an authorizer action reads, leaving out SQLite's own.
fn read_table<'a>(action: &AuthAction<'a>) -> Option<&'a str> {
  match action {
    AuthAction::Read { table_name, .. } if !table_name.is_empty() && !table_name.starts_with("sqlite_") => Some(table_name),
    _ => None,
  }
}

// Whether every name an action touches lies in the namespace, or is SQLite's own.
fn within(prefix: &str, action: &AuthAction<'_>) -> bool {
  let names: Vec<&str> = match *action {
    AuthAction::Read { table_name, .. }
    | AuthAction::Insert { table_name }
    | AuthAction::Update { table_name, .. }
    | AuthAction::Delete { table_name }
    | AuthAction::CreateTable { table_name }
    | AuthAction::DropTable { table_name }
    | AuthAction::AlterTable { table_name, .. }
    | AuthAction::Analyze { table_name } => vec![table_name],
    AuthAction::CreateIndex { index_name, table_name } | AuthAction::DropIndex { index_name, table_name } => vec![index_name, table_name],
    AuthAction::CreateTrigger { trigger_name, table_name } | AuthAction::DropTrigger { trigger_name, table_name } => vec![trigger_name, table_name],
    AuthAction::CreateView { view_name } | AuthAction::DropView { view_name } => vec![view_name],
    // The session recording an op reads the columns of each table a statement
    // changes, so reading its own tables' columns stays within a namespace.
    AuthAction::Pragma { pragma_name: "table_xinfo", pragma_value: Some(table_name) } => vec![table_name],
    AuthAction::Pragma { .. } | AuthAction::Attach { .. } | AuthAction::Detach { .. } | AuthAction::CreateVtable { .. } | AuthAction::DropVtable { .. } => return false,
    AuthAction::CreateTempIndex { .. }
    | AuthAction::CreateTempTable { .. }
    | AuthAction::CreateTempTrigger { .. }
    | AuthAction::CreateTempView { .. }
    | AuthAction::DropTempIndex { .. }
    | AuthAction::DropTempTable { .. }
    | AuthAction::DropTempTrigger { .. }
    | AuthAction::DropTempView { .. } => return false,
    _ => vec![],
  };
  names.into_iter().all(|name| name.is_empty() || name.starts_with(prefix) || name.starts_with("sqlite_"))
}

// A connection as a service sees it: each statement moved into the service's
// namespace and run inside its call window.
pub struct Scoped<'a> {
  pub(crate) connection: &'a Connection,
  pub(crate) access: &'a Shared,
  pub(crate) failure: &'a Failure,
  pub(crate) window: CallWindow,
}

impl Scoped<'_> {
  // Run a statement inside the call window, then restore the window it replaced.
  fn inside<T>(&self, sql: &str, work: impl FnOnce(&str) -> Result<T>) -> Result<T> {
    let sql = match &self.window.prefix {
      Some(prefix) => rewrite(sql, prefix)?,
      None => sql.to_string(),
    };
    let previous = std::mem::replace(&mut self.access.lock().expect("access").window, self.window.clone());
    let result = work(&sql);
    self.access.lock().expect("access").window = previous;
    result
  }
}

impl Db for Scoped<'_> {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    self.inside(sql, |sql| run(self.connection, self.failure, sql, params))
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    self.inside(sql, |sql| all(self.connection, self.failure, sql, params))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::store::Store;
  use serde_json::json;

  fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("space.db")).unwrap();
    store.run_batch("CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT)").unwrap();
    (directory, store)
  }

  fn chat(op: Option<Op>) -> CallWindow {
    CallWindow { prefix: Some("chat/db/".into()), op }
  }

  #[test]
  fn a_service_writes_and_reads_its_own_tables_by_their_plain_names() {
    let (_directory, store) = store();
    let scoped = store.scoped(chat(None));
    scoped.run("CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID; CREATE INDEX note_text ON note (text)", &[]).unwrap();
    scoped.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!("n1"), json!("hi")]).unwrap();
    assert_eq!(scoped.all("SELECT text FROM note", &[]).unwrap(), vec![json!({"text": "hi"})]);
    let names: Vec<Value> = store.all("SELECT name FROM sqlite_schema WHERE name LIKE 'chat/%' ORDER BY name", &[]).unwrap().into_iter().map(|row| row["name"].clone()).collect();
    assert_eq!(names, vec![json!("chat/db/note"), json!("chat/db/note_text")]);
    assert_eq!(store.reader().unwrap().scoped(chat(None)).all("SELECT text FROM note", &[]).unwrap(), vec![json!({"text": "hi"})]);
  }

  #[test]
  fn the_authorizer_refuses_a_service_reaching_outside_its_namespace() {
    let (_directory, store) = store();
    let scoped = store.scoped(chat(None));
    // A service reads the columns of its own tables only, as the session does when an op changes one.
    scoped.run("CREATE TABLE note (id TEXT PRIMARY KEY) WITHOUT ROWID", &[]).unwrap();
    assert_eq!(scoped.all("PRAGMA table_xinfo('chat/db/note')", &[]).unwrap().len(), 1);
    let refused = ["PRAGMA writable_schema = 1", "ATTACH 'other.db' AS other", "PRAGMA table_xinfo('message')", "PRAGMA table_xinfo('notes/db/note')"];
    for sql in refused {
      assert_eq!(scoped.all(sql, &[]).unwrap_err().status, 403, "{sql}");
    }
    // The backstop, for whatever the rewrite might miss: a statement naming another service's table.
    let access = store.access();
    access.lock().unwrap().window = chat(None);
    assert_eq!(store.all("SELECT * FROM message", &[]).unwrap_err().status, 403);
    access.lock().unwrap().window = CallWindow::default();
    assert!(store.all("SELECT * FROM message", &[]).is_ok());
  }

  #[test]
  fn op_id_draws_integer_keys_from_the_op_within_2_to_the_53() {
    let (_directory, store) = store();
    let op = Op::new("POST [chat]/notes".into(), json!({}), None, None, "v".into(), 1, "h".into());
    let scoped = store.scoped(chat(Some(op.clone())));
    scoped.run("CREATE TABLE note (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT) WITHOUT ROWID", &[]).unwrap();
    scoped.run("INSERT INTO note (text) VALUES ('a'), ('b')", &[]).unwrap();
    let ids: Vec<i64> = scoped.all("SELECT id FROM note", &[]).unwrap().into_iter().map(|row| row["id"].as_i64().unwrap()).collect();
    let replica = Op::new("POST [chat]/notes".into(), json!({}), None, None, "v".into(), 1, "h".into());
    let mut expected = vec![op_id(&replica), op_id(&replica)];
    expected.sort();
    assert_eq!(ids, expected);
    assert!(ids.iter().all(|id| (0..1 << 53).contains(id)));
    assert!(store.scoped(chat(None)).run("INSERT INTO note (text) VALUES ('c')", &[]).unwrap_err().message.contains("only inside an op"));
  }
}
