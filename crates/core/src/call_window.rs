//! A call window: for the length of one statement, whether it is a service's
//! and the op it runs within, installed on the connection and then restored.
//!
//! A service's statements run through `Scoped`, inside its call window: each
//! is worked out once (`sql`), SQLite's authorizer refuses anything outside
//! the service's own tables and records what it read (`sql::authorizer`), and
//! `op_id()` draws from the window's op. The core's own statements run outside
//! any window, with neither.

use std::sync::Arc;

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::AuthContext;
use serde_json::Value;

use crate::error::{Result, sqlite};
use crate::sql::Statements;
use crate::sql::authorizer::{Shared, authorize, unrecorded};
use crate::store::{Failure, all, run};
use crate::{Db, Op};

// Whether a statement is a service's, and the op it runs within. The core's
// own statements are neither.
#[derive(Debug, Clone, Default)]
pub struct CallWindow {
  pub service: bool,
  pub op: Option<Op>,
}

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

// A connection as a service sees it: each statement run inside its call window.
pub struct Scoped<'a> {
  pub(crate) connection: &'a Connection,
  pub(crate) access: &'a Shared,
  pub(crate) failure: &'a Failure,
  pub(crate) statements: &'a Statements,
  pub(crate) window: CallWindow,
}

impl Scoped<'_> {
  // Run a statement inside the call window, then restore the window it
  // replaced and settle what it read.
  fn inside<T>(&self, sql: &str, params: &[Value], work: impl FnOnce(&str) -> Result<T>) -> Result<T> {
    let analysed = match self.window.service {
      true => Some(unrecorded(self.access, || self.statements.get(self.connection, self.failure, sql))?),
      false => None,
    };
    let previous = {
      let mut access = self.access.lock().expect("access");
      access.settle(None);
      std::mem::replace(&mut access.window, self.window.clone())
    };
    let result = work(sql);
    let mut access = self.access.lock().expect("access");
    access.window = previous;
    access.settle(analysed.and_then(|analysed| analysed.looked_up(params)));
    result
  }
}

impl Db for Scoped<'_> {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    self.inside(sql, params, |sql| run(self.connection, self.failure, sql, params))
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    self.inside(sql, params, |sql| all(self.connection, self.failure, sql, params))
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
    CallWindow { service: true, op }
  }

  #[test]
  fn a_service_writes_and_reads_its_own_tables_as_written() {
    let (_directory, store) = store();
    let scoped = store.scoped(chat(None));
    scoped.run("CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID; CREATE INDEX note_text ON note (text)", &[]).unwrap();
    scoped.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!("n1"), json!("hi")]).unwrap();
    assert_eq!(scoped.all("SELECT text FROM note", &[]).unwrap(), vec![json!({"text": "hi"})]);
    // SQLite's shorthand for a list of a table's rows, which a parser would have to know.
    assert_eq!(scoped.all("SELECT text FROM note WHERE text IN note_texts", &[]).unwrap_err().status, 500);
    scoped.run("CREATE TABLE note_texts (text TEXT PRIMARY KEY) WITHOUT ROWID; INSERT INTO note_texts VALUES ('hi')", &[]).unwrap();
    assert_eq!(scoped.all("SELECT text FROM note WHERE text IN note_texts", &[]).unwrap(), vec![json!({"text": "hi"})]);
    assert_eq!(store.reader().unwrap().scoped(chat(None)).all("SELECT text FROM note", &[]).unwrap(), vec![json!({"text": "hi"})]);
  }

  #[test]
  fn the_authorizer_refuses_a_service_reaching_the_cores_tables_or_another_file() {
    let (_directory, store) = store();
    store.run_batch("CREATE TABLE carbon_result (hash TEXT PRIMARY KEY)").unwrap();
    let scoped = store.scoped(chat(None));
    // A service reads the columns of its own tables, as the session does when an op changes one.
    scoped.run("CREATE TABLE note (id TEXT PRIMARY KEY) WITHOUT ROWID", &[]).unwrap();
    assert_eq!(scoped.all("PRAGMA table_xinfo('note')", &[]).unwrap().len(), 1);
    let refused = ["PRAGMA writable_schema = 1", "ATTACH 'other.db' AS other", "PRAGMA table_xinfo('carbon_result')", "SELECT * FROM carbon_result", "DELETE FROM carbon_result"];
    for sql in refused {
      assert_eq!(scoped.all(sql, &[]).unwrap_err().status, 403, "{sql}");
    }
    assert!(store.all("SELECT * FROM carbon_result", &[]).is_ok(), "the core's own statements reach its tables");
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
