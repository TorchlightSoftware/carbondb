//! The check every service's schema passes after its `init`: each of its
//! tables takes its rows' keys from the op that creates them, never from the
//! rows already in the table.
//!
//! Every device applies ops in one shared order and rewinds them when an
//! earlier op arrives late. A key SQLite assigns, such as a rowid or an
//! AUTOINCREMENT id, depends on the rows already there, so after a rewind the
//! row its author saw as 2 can become 3, and a later op naming 2 changes
//! another row. A key the op supplies is the same on every device. SQLite's
//! sessions, the record of each op's writes, also skip a table with no primary
//! key, so a rewind would silently miss it.

use serde_json::{Value, json};

use crate::Db;
use crate::error::{Result, bad_request};

// Refuse the first table in a namespace whose rows' keys SQLite assigns.
pub fn check_keys(db: &dyn Db, prefix: &str) -> Result<()> {
  let tables = db.all("SELECT name, wr FROM pragma_table_list WHERE type = 'table' AND substr(name, 1, length(?)) = ?", &[json!(prefix), json!(prefix)])?;
  for table in &tables {
    let name = table["name"].as_str().unwrap_or_default();
    let keys = db.all("SELECT type FROM pragma_table_info(?) WHERE pk > 0", &[json!(name)])?;
    if let Some(problem) = problem(table["wr"] == json!(1), &keys) {
      return Err(guidance(name.strip_prefix(prefix).unwrap_or(name), problem));
    }
  }
  Ok(())
}

// What is wrong with a table's key, given whether it is WITHOUT ROWID and the types of its key columns.
fn problem(without_rowid: bool, keys: &[Value]) -> Option<&'static str> {
  let rowid_alias = !without_rowid && keys.len() == 1 && keys[0]["type"].as_str().is_some_and(|kind| kind.eq_ignore_ascii_case("INTEGER"));
  match (keys.is_empty(), rowid_alias) {
    (true, _) => Some("has no primary key"),
    (false, true) => Some("has an INTEGER PRIMARY KEY, which is SQLite's rowid: SQLite assigns it, and AUTOINCREMENT too, from the rows already in the table"),
    _ => None,
  }
}

fn guidance(table: &str, problem: &str) -> crate::error::ServiceError {
  bad_request(format!(
    "Table '{table}' {problem}. CarbonDB applies ops in one shared order on every device and rewinds them when an earlier op arrives late, \
     so a key SQLite assigns can differ between devices and move under the row's author, and a table without a primary key is missed by the record a rewind undoes. \
     A row's key must come from the op that creates it: declare the table WITHOUT ROWID with a PRIMARY KEY the handler fills from op.id(), \
     or, for an integer key, `id INTEGER PRIMARY KEY DEFAULT (op_id())`."
  ))
  .with_details(json!({"table": table}))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::call_window::CallWindow;
  use crate::store::Store;

  #[test]
  fn refuses_a_table_whose_keys_sqlite_assigns_and_says_how_to_fix_it() {
    let cases = [
      ("CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID", None),
      ("CREATE TABLE note (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT) WITHOUT ROWID", None),
      ("CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT)", None),
      ("CREATE TABLE note (a TEXT, b TEXT, PRIMARY KEY (a, b))", None),
      ("CREATE TABLE note (text TEXT)", Some("has no primary key")),
      ("CREATE TABLE note (id INTEGER PRIMARY KEY, text TEXT)", Some("INTEGER PRIMARY KEY")),
      ("CREATE TABLE note (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT)", Some("INTEGER PRIMARY KEY")),
    ];
    for (sql, expected) in cases {
      let directory = tempfile::tempdir().unwrap();
      let store = Store::open(&directory.path().join("s.db")).unwrap();
      store.scoped(CallWindow { prefix: Some("notes/db/".into()), op: None }).run(sql, &[]).unwrap();
      store.run_batch("CREATE TABLE elsewhere (text TEXT)").unwrap();
      let checked = check_keys(&store, "notes/db/");
      match expected {
        None => assert!(checked.is_ok(), "{sql}: {checked:?}"),
        Some(problem) => {
          let error = checked.unwrap_err();
          assert!(error.message.starts_with("Table 'note' ") && error.message.contains(problem) && error.message.contains("op_id()"), "{sql}: {error}");
          assert_eq!(error.details, Some(json!({"table": "note"})));
        }
      }
    }
  }
}
