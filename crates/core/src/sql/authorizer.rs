//! The authorizer: what a statement may touch, and what it read.
//!
//! SQLite asks the authorizer about every table, column and pragma a
//! statement names as it is compiled. A service's tables share their file with
//! the core's own record of the space (the op log, results and migrations,
//! all named `carbon_`), so a service's statement that names one of those, or
//! reaches another file through ATTACH or a pragma, is refused; the core's
//! own statements, outside any call window, reach everything.
//!
//! The authorizer also records what statements read: a GET's read set, which
//! a watch re-runs on, and an op's, which rewind checks. It reports tables,
//! so each statement's reads are settled once it has run: a statement that
//! `lookup` finds looking up one row by its key, and that read only that
//! row's table, read that row; any other read the tables it read. What an op
//! writes comes from its changeset, in `store::changeset`.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use rusqlite::hooks::{AuthAction, Authorization};
use serde::{Deserialize, Serialize};

use crate::sql::call_window::CallWindow;
use crate::store::changeset::Row;

// What a statement read: a whole table, or the one row of it looked up by its whole key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Read {
  Table(String),
  Row(Row),
}

impl Read {
  pub fn table(&self) -> &str {
    match self {
      Read::Table(table) => table,
      Read::Row(row) => &row.table,
    }
  }
}

// What the statements on one connection may touch, and what they have read:
// the tables the statement running now has read, and what the statements
// settled since recording started read. SQLite's authorizer and functions
// must be Send, so it sits behind a mutex.
#[derive(Default)]
pub(crate) struct Access {
  pub(crate) window: CallWindow,
  reading: bool,
  statement: BTreeSet<String>,
  read: BTreeSet<Read>,
}

pub(crate) type Shared = Arc<Mutex<Access>>;

pub(crate) fn start_reading(access: &Shared) {
  let mut access = access.lock().expect("access");
  access.reading = true;
  access.statement.clear();
  access.read.clear();
}

impl Access {
  // Settle the statement that just ran: what it read joins the record, as the
  // one row it looked up when the only table it read is that row's, or else
  // as the tables it read.
  pub(crate) fn settle(&mut self, looked_up: Option<Row>) {
    let tables = std::mem::take(&mut self.statement);
    match looked_up {
      Some(row) if tables.len() == 1 && tables.contains(&row.table) => {
        self.read.insert(Read::Row(row));
      }
      _ => self.read.extend(tables.into_iter().map(Read::Table)),
    }
  }

  // What the statements since recording started read, settling the last.
  pub(crate) fn read(&mut self) -> &BTreeSet<Read> {
    self.settle(None);
    &self.read
  }
}

// Stop recording and return what was read.
pub(crate) fn stop_reading(access: &Shared) -> Vec<Read> {
  let mut access = access.lock().expect("access");
  access.settle(None);
  access.reading = false;
  std::mem::take(&mut access.read).into_iter().collect()
}

// Run the core's own work on a connection without recording what it reads.
pub(crate) fn unrecorded<T>(access: &Shared, work: impl FnOnce() -> T) -> T {
  let reading = std::mem::replace(&mut access.lock().expect("access").reading, false);
  let done = work();
  access.lock().expect("access").reading = reading;
  done
}

// Record what a statement reads, and refuse a service's statement that reaches
// outside its own tables.
pub(crate) fn authorize(access: &mut Access, action: &AuthAction<'_>) -> Authorization {
  if let Some(table) = read_table(action).filter(|_| access.reading) {
    access.statement.insert(table.to_string());
  }
  match access.window.service && !within(action) {
    true => Authorization::Deny,
    false => Authorization::Allow,
  }
}

// The table an authorizer action reads, leaving out SQLite's own.
fn read_table<'a>(action: &AuthAction<'a>) -> Option<&'a str> {
  match action {
    AuthAction::Read { table_name, .. } if !table_name.is_empty() && !table_name.starts_with("sqlite_") => Some(table_name),
    _ => None,
  }
}

// Whether every name an action touches is the service's own, or SQLite's: never the core's.
fn within(action: &AuthAction<'_>) -> bool {
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
    // changes, so reading its own tables' columns is the service's own.
    AuthAction::Pragma { pragma_name: "table_xinfo" | "table_info", pragma_value: Some(table_name) } => vec![table_name],
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
  names.into_iter().all(|name| !name.starts_with(CORE))
}

// What the core's own tables in a space's file are named.
pub(crate) const CORE: &str = "carbon_";

