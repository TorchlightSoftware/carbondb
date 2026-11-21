//! Reading a changeset: the rows an op's transaction changed, each named by
//! its table and primary key, which are the op's write set, and the tables
//! they belong to, which watches re-run on.
//!
//! A changeset is also a row's history. It holds a deleted row whole, and for
//! an update the old values of the columns it changed, so undoing the changes
//! later ops made to a row, newest first, gives the row as it stood before
//! them (`row_as_of`), as a rewind undoes whole changesets.

use rusqlite::fallible_streaming_iterator::FallibleStreamingIterator;
use rusqlite::hooks::Action;
use rusqlite::session::{ChangesetItem, ChangesetIter};
use rusqlite::types::Value as Sql;
use serde_json::Value;

use crate::error::{Result, sqlite};
use crate::store::from_sql;

// A row a changeset changes: its table, and its primary key's values as JSON
// text. Every table a service owns has a primary key the op chose, so a row
// is the same row on every replica.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Row {
  pub table: String,
  pub key: String,
}

// The rows a changeset changes, in table and key order.
pub fn rows(changeset: &[u8]) -> Result<Vec<Row>> {
  let input: &mut dyn std::io::Read = &mut &changeset[..];
  let mut items = ChangesetIter::start_strm(&input).map_err(sqlite)?;
  let mut rows = std::collections::BTreeSet::new();
  while let Some(item) = items.next().map_err(sqlite)? {
    rows.insert(changed_row(item).map_err(sqlite)?);
  }
  Ok(rows.into_iter().collect())
}

// The row one change touches, its key's values in key order, as a read
// names it: an insert names it by its new key, an update or a delete by its
// old one. A changeset marks each key column with its place in the key.
fn changed_row(item: &ChangesetItem) -> rusqlite::Result<Row> {
  let op = item.op()?;
  let value = |column: usize| match op.code() {
    Action::SQLITE_INSERT => item.new_value(column),
    _ => item.old_value(column),
  };
  let mut keys: Vec<(u8, usize)> = item.pk()?.iter().enumerate().filter(|(_, in_key)| **in_key != 0).map(|(column, in_key)| (*in_key, column)).collect();
  keys.sort();
  let key = keys.into_iter().map(|(_, column)| Ok(from_sql(Sql::try_from(value(column)?)?))).collect::<rusqlite::Result<Vec<Value>>>()?;
  Ok(Row { table: op.table_name().to_string(), key: Value::Array(key).to_string() })
}

// A row's values, in its table's column order, as SQLite holds them.
pub type Values = Vec<Sql>;

// What one op's changeset did to one row: what undoing it needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
  Inserted,
  // The row as it was.
  Deleted(Values),
  // Each column's value before, where the op changed it.
  Updated(Vec<Option<Sql>>),
}

// The change a changeset made to a row, if it made one.
pub fn change_to(changeset: &[u8], row: &Row) -> Result<Option<Change>> {
  let input: &mut dyn std::io::Read = &mut &changeset[..];
  let mut items = ChangesetIter::start_strm(&input).map_err(sqlite)?;
  while let Some(item) = items.next().map_err(sqlite)? {
    if changed_row(item).map_err(sqlite)? == *row {
      return change_of(item).map(Some).map_err(sqlite);
    }
  }
  Ok(None)
}

fn change_of(item: &ChangesetItem) -> rusqlite::Result<Change> {
  let op = item.op()?;
  let columns = 0..op.number_of_columns() as usize;
  Ok(match op.code() {
    Action::SQLITE_INSERT => Change::Inserted,
    Action::SQLITE_DELETE => Change::Deleted(columns.map(|column| Ok(Sql::try_from(item.old_value(column)?)?)).collect::<rusqlite::Result<_>>()?),
    // An update holds no old value for a column it left as it was.
    _ => Change::Updated(columns.map(|column| item.old_value(column).ok().map(Sql::try_from).transpose()).collect::<std::result::Result<_, _>>()?),
  })
}

// The row before a change, given the row after it. A column added to the
// table since the change keeps its value after it.
pub fn undo(after: Option<Values>, change: &Change) -> Option<Values> {
  match change {
    Change::Inserted => None,
    Change::Deleted(before) => Some(before.clone()),
    Change::Updated(before) => after.map(|row| row.into_iter().enumerate().map(|(column, now)| before.get(column).cloned().flatten().unwrap_or(now)).collect()),
  }
}

// A row as it stood before a run of changes: as it is now, with each change
// undone, newest first.
pub fn row_as_of(now: Option<Values>, newest_first: impl IntoIterator<Item = Change>) -> Option<Values> {
  newest_first.into_iter().fold(now, |row, change| undo(row, &change))
}

// The tables a changeset changes, in name order.
pub fn tables(changeset: &[u8]) -> Result<Vec<String>> {
  let tables: std::collections::BTreeSet<String> = rows(changeset)?.into_iter().map(|row| row.table).collect();
  Ok(tables.into_iter().collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::store::Store;

  // The changeset of one write to a table declared as given, after a first row.
  fn written(table: &str, first: &str, write: &str) -> Vec<u8> {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("space.db")).unwrap();
    store.run_batch(&format!("{table}; {first}")).unwrap();
    store.write(|store| store.run_batch(write)).unwrap().1
  }

  // A read names a row by its key in key order (`sql::lookup`); a write must
  // name it the same way, or a read and a write of one row never meet.
  fn text(value: &str) -> Sql {
    Sql::Text(value.into())
  }

  #[test]
  fn undoing_a_change_restores_the_row_before_it() {
    let row = |role: &str| vec![text("ann"), text(role), Sql::Integer(1)];
    let cases = [
      ("an insert leaves no row", Some(row("member")), Change::Inserted, None),
      ("a delete restores the whole row", None, Change::Deleted(row("inviter")), Some(row("inviter"))),
      ("an update restores only what it changed", Some(row("member")), Change::Updated(vec![None, Some(text("inviter")), None]), Some(row("inviter"))),
      ("a column added since keeps its value", Some([row("member"), vec![text("added")]].concat()), Change::Updated(vec![None, Some(text("inviter")), None]), Some([row("inviter"), vec![text("added")]].concat())),
    ];
    for (name, after, change, before) in cases {
      assert_eq!(undo(after, &change), before, "{name}");
    }
  }

  #[test]
  fn a_changeset_tells_what_it_did_to_one_row_and_nothing_of_a_row_it_left() {
    let table = "CREATE TABLE member (id TEXT PRIMARY KEY, role TEXT, joined INTEGER) WITHOUT ROWID";
    let first = "INSERT INTO member VALUES ('ann', 'member', 1), ('bob', 'member', 2)";
    let ann = Row { table: "member".into(), key: r#"["ann"]"#.into() };
    let cases = [
      ("an insert", "INSERT INTO member VALUES ('ann', 'member', 1)", Some(Change::Inserted)),
      ("an update", "UPDATE member SET role = 'inviter' WHERE id = 'ann'", Some(Change::Updated(vec![Some(text("ann")), Some(text("member")), None]))),
      ("a delete", "DELETE FROM member WHERE id = 'ann'", Some(Change::Deleted(vec![text("ann"), text("member"), Sql::Integer(1)]))),
      ("another row", "UPDATE member SET role = 'inviter' WHERE id = 'bob'", None),
    ];
    for (name, write, expected) in cases {
      let first = if name == "an insert" { "SELECT 1" } else { first };
      assert_eq!(change_to(&written(table, first, write), &ann).unwrap(), expected, "{name}");
    }
  }

  #[test]
  fn a_row_is_named_by_its_key_values_in_key_order_whatever_order_its_columns_are_declared_in() {
    let changeset = written("CREATE TABLE grant (person TEXT, role TEXT, PRIMARY KEY (role, person)) WITHOUT ROWID", "SELECT 1", "INSERT INTO grant VALUES ('ann', 'inviter')");
    assert_eq!(rows(&changeset).unwrap(), [Row { table: "grant".into(), key: r#"["inviter","ann"]"#.into() }]);
  }
}
