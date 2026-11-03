//! Spike: can the Rust core own a SQLite transaction while an async JS handler
//! runs inside it?
//!
//! Everything happens on the JS thread. `apply` opens the transaction, calls the
//! handler, and chains commit and rollback onto the Promise it returns, so Rust
//! never blocks waiting for JS. While the Promise is pending, the handler's
//! synchronous `run` and `all` calls re-enter Rust and use the same connection,
//! inside the open transaction.
//!
//! `read` answers from a second, read-only connection in WAL mode, as GETs and
//! watch re-runs will, and reports the tables the query read, from SQLite's
//! authorizer, as watch's read set.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OpenFlags, params_from_iter};
use serde_json::{Map, Value};

// Turn any rusqlite failure into a JS error at the boundary.
fn to_napi(error: rusqlite::Error) -> Error {
  Error::from_reason(error.to_string())
}

// A JS parameter as a SQLite value. rusqlite's own serde_json binding stores a
// string as JSON text, quotes and all, so strings and numbers map directly here,
// and only objects and arrays become JSON text.
fn json_to_sql(value: Value) -> rusqlite::types::Value {
  use rusqlite::types::Value as Sql;
  match value {
    Value::Null => Sql::Null,
    Value::Bool(flag) => Sql::Integer(flag as i64),
    Value::Number(number) => number.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(number.as_f64().unwrap_or_default())),
    Value::String(text) => Sql::Text(text),
    nested => Sql::Text(nested.to_string()),
  }
}

fn sql_params(params: Option<Vec<Value>>) -> Vec<rusqlite::types::Value> {
  params.unwrap_or_default().into_iter().map(json_to_sql).collect()
}

// Each row as an object keyed by column name.
fn query_rows(conn: &Connection, sql: &str, params: Vec<rusqlite::types::Value>) -> rusqlite::Result<Vec<Value>> {
  let mut statement = conn.prepare(sql)?;
  let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
  let rows = statement.query_map(params_from_iter(params), |row| {
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
      let value: rusqlite::types::Value = row.get(index)?;
      object.insert(column.clone(), sql_to_json(value));
    }
    Ok(Value::Object(object))
  })?;
  rows.collect()
}

fn sql_to_json(value: rusqlite::types::Value) -> Value {
  use rusqlite::types::Value as Sql;
  match value {
    Sql::Null => Value::Null,
    Sql::Integer(number) => Value::from(number),
    Sql::Real(number) => Value::from(number),
    Sql::Text(text) => Value::from(text),
    Sql::Blob(bytes) => Value::from(bytes),
  }
}

// The tables a statement reads, as SQLite's authorizer reports them while it prepares it.
// The hook must be Send, so the set it fills sits behind an Arc and a Mutex.
fn tables_read(conn: &Connection, sql: &str) -> rusqlite::Result<Vec<String>> {
  let read = Arc::new(Mutex::new(BTreeSet::new()));
  let collector = Arc::clone(&read);
  conn.authorizer(Some(move |context: AuthContext<'_>| {
    if let AuthAction::Read { table_name, .. } = context.action {
      collector.lock().expect("authorizer set").insert(table_name.to_string());
    }
    Authorization::Allow
  }))?;
  let prepared = conn.prepare(sql).map(|_| ());
  conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
  prepared?;
  let tables = std::mem::take(&mut *read.lock().expect("authorizer set"));
  Ok(tables.into_iter().collect())
}

#[napi(object)]
pub struct Read {
  pub rows: Vec<Value>,
  pub tables: Vec<String>,
}

#[napi]
pub struct Space {
  writer: Rc<Connection>,
  reader: Connection,
  applying: Rc<Cell<bool>>,
}

#[napi]
impl Space {
  // A space's database file, with one writer and one read-only reader.
  #[napi(constructor)]
  pub fn new(path: String) -> Result<Self> {
    let writer = Connection::open(&path).map_err(to_napi)?;
    writer.pragma_update(None, "journal_mode", "wal").map_err(to_napi)?;
    let reader = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(to_napi)?;
    Ok(Space { writer: Rc::new(writer), reader, applying: Rc::new(Cell::new(false)) })
  }

  // A write from the handler, inside whatever transaction is open.
  #[napi]
  pub fn run(&self, sql: String, params: Option<Vec<Value>>) -> Result<u32> {
    let changed = self.writer.execute(&sql, params_from_iter(sql_params(params))).map_err(to_napi)?;
    Ok(changed as u32)
  }

  // A query from the handler, which sees the transaction's own uncommitted writes.
  #[napi]
  pub fn all(&self, sql: String, params: Option<Vec<Value>>) -> Result<Vec<Value>> {
    query_rows(&self.writer, &sql, sql_params(params)).map_err(to_napi)
  }

  // A read from the read-only connection: committed state only, with its read set.
  #[napi]
  pub fn read(&self, sql: String, params: Option<Vec<Value>>) -> Result<Read> {
    let tables = tables_read(&self.reader, &sql).map_err(to_napi)?;
    let rows = query_rows(&self.reader, &sql, sql_params(params)).map_err(to_napi)?;
    Ok(Read { rows, tables })
  }

  // Run an async handler in one transaction: commit when its Promise resolves,
  // roll back when it rejects or throws, and settle with its result or error.
  // The catch step returns a rejected promise, so the Rust type nests one
  // promise in another; JS flattens them, and the caller sees Promise<unknown>.
  #[napi(ts_return_type = "Promise<unknown>")]
  pub fn apply<'env>(&self, handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>) -> Result<PromiseRaw<'env, PromiseRaw<'env, Unknown<'env>>>> {
    if self.applying.replace(true) {
      return Err(Error::from_reason("A transaction is already open on this space"));
    }
    if let Err(error) = self.writer.execute_batch("BEGIN") {
      self.applying.set(false);
      return Err(to_napi(error));
    }

    let pending = match handler.call(()) {
      Ok(pending) => pending,
      // The handler's error is the one the caller needs; a failed rollback here
      // could only mean the transaction had already ended.
      Err(error) => {
        let _ = finish(&self.writer, &self.applying, "ROLLBACK");
        return Err(error);
      }
    };

    let (writer, applying) = (Rc::clone(&self.writer), Rc::clone(&self.applying));
    let committed = pending.then(move |context| {
      finish(&writer, &applying, "COMMIT").map_err(to_napi)?;
      Ok(context.value)
    })?;

    // Rejecting with the original value, not an Err, keeps the caller's error
    // intact: napi-rs turns an Err from a promise callback into a fresh Error,
    // which would drop a service error's status, code and details.
    let (writer, applying) = (Rc::clone(&self.writer), Rc::clone(&self.applying));
    committed.catch(move |context: CallbackContext<Unknown<'env>>| {
      if applying.get() {
        let _ = finish(&writer, &applying, "ROLLBACK");
      }
      PromiseRaw::<Unknown<'env>>::reject(&context.env, context.value)
    })
  }
}

// End the open transaction and mark the space free for the next one.
fn finish(writer: &Connection, applying: &Cell<bool>, statement: &str) -> rusqlite::Result<()> {
  let result = writer.execute_batch(statement);
  applying.set(false);
  result
}
