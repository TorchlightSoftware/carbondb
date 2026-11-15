//! Reading a changeset: the rows an op's transaction changed, each named by
//! its table and primary key, which are the op's write set, and the tables
//! they belong to, which watches re-run on.

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

// The row one change touches: an insert names it by its new key, an update
// or a delete by its old one.
fn changed_row(item: &ChangesetItem) -> rusqlite::Result<Row> {
  let op = item.op()?;
  let value = |column: usize| match op.code() {
    Action::SQLITE_INSERT => item.new_value(column),
    _ => item.old_value(column),
  };
  let keys = item.pk()?.iter().enumerate().filter(|(_, in_key)| **in_key != 0);
  let key = keys.map(|(column, _)| Ok(from_sql(Sql::try_from(value(column)?)?))).collect::<rusqlite::Result<Vec<Value>>>()?;
  Ok(Row { table: op.table_name().to_string(), key: Value::Array(key).to_string() })
}

// The tables a changeset changes, in name order.
pub fn tables(changeset: &[u8]) -> Result<Vec<String>> {
  let tables: std::collections::BTreeSet<String> = rows(changeset)?.into_iter().map(|row| row.table).collect();
  Ok(tables.into_iter().collect())
}
