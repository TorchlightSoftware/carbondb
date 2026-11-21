//! Can a `callsync` read rebuild a row as it stood at a place from the
//! changesets a store already keeps for rewinds, instead of reading a WAL
//! kept without checkpoints? How does the reader's statement then run against
//! the rebuilt row, and what does each way cost?
//!
//! `row_as_of` rebuilds: the row as it is now, with the change each op placed
//! after the reader made to it undone, newest first, as a rewind undoes ops.
//! `in_temp_table`, `in_cte` and `Scratch` are three ways to run a statement
//! against the rebuilt row. `history` keeps a store as the core keeps one, and
//! reads a row as of a place both ways, for the tests and for `main`.

use fallible_streaming_iterator::FallibleStreamingIterator;
use rusqlite::hooks::Action;
use rusqlite::session::{ChangesetItem, ChangesetIter};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, Result, params_from_iter};

pub mod history;

// A row's values, in its table's column order.
pub type Columns = Vec<Value>;

// What a statement answered: its rows, each its values in order.
pub type Rows = Vec<Columns>;

// What one op's changeset did to one row: what undoing it needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
  Inserted,
  // The row as it was.
  Deleted(Columns),
  // Each column's value before, where the op changed it.
  Updated(Vec<Option<Value>>),
}

// The row before a change, given the row after it.
pub fn undo(after: Option<Columns>, change: &Change) -> Option<Columns> {
  match change {
    Change::Inserted => None,
    Change::Deleted(before) => Some(before.clone()),
    Change::Updated(before) => after.map(|row| row.into_iter().zip(before).map(|(now, was)| was.clone().unwrap_or(now)).collect()),
  }
}

// A row as it stood at a place: the row now, with each later change to it undone, newest first.
pub fn row_as_of(now: Option<Columns>, newest_first: impl IntoIterator<Item = Change>) -> Option<Columns> {
  newest_first.into_iter().fold(now, |row, change| undo(row, &change))
}

// The change a changeset made to one row, named by its table and key, if it made one.
pub fn change_to(changeset: &[u8], table: &str, key: &[Value]) -> Result<Option<Change>> {
  let input: &mut dyn std::io::Read = &mut &changeset[..];
  let mut items = ChangesetIter::start_strm(&input)?;
  while let Some(item) = items.next()? {
    let op = item.op()?;
    if op.table_name() != table || named(item)? != key {
      continue;
    }
    let columns = 0..op.number_of_columns() as usize;
    return Ok(Some(match op.code() {
      Action::SQLITE_INSERT => Change::Inserted,
      Action::SQLITE_DELETE => Change::Deleted(columns.map(|column| owned(item.old_value(column))).collect::<Result<_>>()?),
      // An update holds a column's old value only where it changed it.
      _ => Change::Updated(columns.map(|column| item.old_value(column).ok().map(|value| owned(Ok(value))).transpose()).collect::<Result<_>>()?),
    }));
  }
  Ok(None)
}

// The key of the row a changeset item changes: an insert names it by its new
// values, an update or delete by its old ones.
pub(crate) fn named(item: &ChangesetItem) -> Result<Columns> {
  let op = item.op()?;
  let inserted = matches!(op.code(), Action::SQLITE_INSERT);
  let pk = item.pk()?;
  (0..op.number_of_columns() as usize).filter(|&column| pk[column] != 0).map(|column| owned(if inserted { item.new_value(column) } else { item.old_value(column) })).collect()
}

// A value a changeset holds, owned.
fn owned(value: Result<ValueRef<'_>>) -> Result<Value> {
  Ok(Value::try_from(value?)?)
}

// A table as running a statement against a rebuilt row needs it: its name,
// the statement that declared it, its columns, and its key's columns.
#[derive(Debug, Clone)]
pub struct Table {
  pub name: String,
  pub create: String,
  pub columns: Vec<String>,
  pub key: Vec<String>,
}

impl Table {
  // A table as a store declares it.
  pub fn of(connection: &Connection, name: &str) -> Result<Table> {
    let create = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?", [name], |row| row.get(0))?;
    let mut info = connection.prepare("SELECT name, pk FROM pragma_table_info(?) ORDER BY cid")?;
    let columns: Vec<(String, i64)> = info.query_map([name], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_>>()?;
    let mut key: Vec<&(String, i64)> = columns.iter().filter(|(_, pk)| *pk > 0).collect();
    key.sort_by_key(|(_, pk)| *pk);
    Ok(Table { name: name.into(), create, key: key.into_iter().map(|(column, _)| column.clone()).collect(), columns: columns.iter().map(|(column, _)| column.clone()).collect() })
  }

  // The row a key names, as the table holds it now.
  pub fn row(&self, connection: &Connection, key: &[Value]) -> Result<Option<Columns>> {
    let found = query(connection, &format!("SELECT * FROM main.{} WHERE {}", self.name, self.matching()), key)?;
    Ok(found.into_iter().next())
  }

  fn matching(&self) -> String {
    self.key.iter().map(|column| format!("{column} = ?")).collect::<Vec<_>>().join(" AND ")
  }

  fn holes(&self) -> String {
    vec!["?"; self.columns.len()].join(", ")
  }

  // Put a row, when there is one, into a table declared as this one.
  fn insert(&self, connection: &Connection, into: &str, row: Option<&Columns>) -> Result<()> {
    if let Some(row) = row {
      connection.execute(&format!("INSERT INTO {into} VALUES ({})", self.holes()), params_from_iter(row))?;
    }
    Ok(())
  }
}

// Every row a statement answers.
pub fn query(connection: &Connection, sql: &str, params: &[Value]) -> Result<Rows> {
  let mut statement = connection.prepare_cached(sql)?;
  let width = statement.column_count();
  statement.query_map(params_from_iter(params), |row| (0..width).map(|column| row.get(column)).collect())?.collect()
}

// Run a statement against a rebuilt row in a temporary table on the reader's
// own connection, declared as the store's table and named the same: SQLite
// finds a temporary table before the store's.
pub fn in_temp_table(reader: &Connection, table: &Table, row: Option<&Columns>, sql: &str, params: &[Value]) -> Result<Rows> {
  reader.execute_batch(&table.create.replacen("CREATE TABLE", "CREATE TEMP TABLE", 1))?;
  let found = table.insert(reader, &format!("temp.{}", table.name), row).and_then(|_| query(reader, sql, params));
  reader.execute_batch(&format!("DROP TABLE temp.{}", table.name))?;
  found
}

// Run a statement against a rebuilt row through a common table expression
// named for the table, put before the statement; its columns are bound first.
pub fn in_cte(reader: &Connection, table: &Table, row: Option<&Columns>, sql: &str, params: &[Value]) -> Result<Rows> {
  let rows = match row {
    Some(_) => format!("VALUES ({})", table.holes()),
    None => format!("SELECT {} WHERE 0", vec!["NULL"; table.columns.len()].join(", ")),
  };
  let wrapped = format!("WITH {}({}) AS ({rows}) {sql}", table.name, table.columns.join(", "));
  let bound: Vec<Value> = row.into_iter().flatten().chain(params).cloned().collect();
  query(reader, &wrapped, &bound)
}

// A connection in memory holding only a table declared as the store's, kept
// for reading rebuilt rows of it one at a time.
pub struct Scratch {
  connection: Connection,
  table: Table,
}

impl Scratch {
  pub fn new(table: &Table) -> Result<Scratch> {
    let connection = Connection::open_in_memory()?;
    connection.execute_batch(&table.create)?;
    Ok(Scratch { connection, table: table.clone() })
  }

  // Run a statement against a rebuilt row, the only row the table then holds.
  pub fn read(&self, row: Option<&Columns>, sql: &str, params: &[Value]) -> Result<Rows> {
    self.connection.execute_batch(&format!("DELETE FROM {}", self.table.name))?;
    self.table.insert(&self.connection, &self.table.name, row)?;
    query(&self.connection, sql, params)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::history::{MEMBER, Store};

  fn text(value: &str) -> Value {
    Value::Text(value.into())
  }

  #[test]
  fn undoing_a_change_restores_the_row_before_it() {
    let row = |role: &str| vec![text("ann"), text("Ann"), text(role), Value::Integer(1)];
    let cases = [
      ("an insert leaves no row", Some(row("member")), Change::Inserted, None),
      ("a delete restores the whole row", None, Change::Deleted(row("inviter")), Some(row("inviter"))),
      ("an update restores only what it changed", Some(row("member")), Change::Updated(vec![None, None, Some(text("inviter")), None]), Some(row("inviter"))),
    ];
    for (name, after, change, before) in cases {
      assert_eq!(undo(after, &change), before, "{name}");
    }
  }

  // Ann joins, changes role, changes name, leaves, joins again, and changes
  // both; Bob joins at 3. Read as of each place, her row is as it stood then,
  // both from the kept WAL and rebuilt from the changesets of later ops.
  #[test]
  fn a_row_rebuilt_from_later_changesets_is_the_row_the_kept_wal_holds_at_each_place() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::create(&directory.path().join("members.db"), MEMBER).unwrap();
    let ops = [
      "INSERT INTO member VALUES ('ann', 'Ann', 'member', 1)",
      "UPDATE member SET role = 'inviter' WHERE id = 'ann'",
      "UPDATE member SET name = 'Annie' WHERE id = 'ann'; INSERT INTO member VALUES ('bob', 'Bob', 'member', 3)",
      "DELETE FROM member WHERE id = 'ann'",
      "INSERT INTO member VALUES ('ann', 'Ann', 'member', 5)",
      "UPDATE member SET name = 'ANN', role = 'manager' WHERE id = 'ann'",
    ];
    let table = Table::of(&store.reader().unwrap(), "member").unwrap();
    let ann = [text("ann")];
    let mut stood = vec![None];
    for (time, sql) in (1..).zip(ops) {
      store.apply(time, sql).unwrap();
      stood.push(table.row(&store.reader().unwrap(), &ann).unwrap());
    }
    let reader = store.reader().unwrap();
    let everything = "SELECT * FROM member WHERE id = ?";
    for (place, expected) in stood.iter().enumerate() {
      let expected: Rows = expected.iter().cloned().collect();
      let kept = store.read_at_snapshot(&reader, &ann, place as i64, everything, &ann).unwrap();
      let rebuilt = store.read_rebuilt(&reader, &table, &ann, place as i64, |row| in_temp_table(&reader, &table, row, everything, &ann)).unwrap();
      assert_eq!((kept, rebuilt), (expected.clone(), expected), "as of {place}");
    }
  }

  // A statement means what it means against the store's table only where the
  // rebuilt row sits in a table declared the same: a column's type affinity
  // and collation are part of its declaration.
  #[test]
  fn a_statement_answers_against_a_rebuilt_row_as_against_the_store_only_in_a_table_declared_the_same() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::create(&directory.path().join("members.db"), MEMBER).unwrap();
    store.apply(1, "INSERT INTO member VALUES ('5', 'Ann', 'inviter', 1)").unwrap();
    let reader = store.reader().unwrap();
    let table = Table::of(&reader, "member").unwrap();
    let row = table.row(&reader, &[text("5")]).unwrap();
    let scratch = Scratch::new(&table).unwrap();
    let statements = [
      ("a number matches a text key by the column's affinity", "SELECT role FROM member WHERE id = ?", vec![Value::Integer(5)]),
      ("a name compares by the column's collation", "SELECT name = 'ANN' FROM member WHERE id = ?", vec![text("5")]),
    ];
    for (name, sql, params) in statements {
      let store_answers = query(&reader, sql, &params).unwrap();
      let answers = [
        ("a temporary table", in_temp_table(&reader, &table, row.as_ref(), sql, &params).unwrap(), true),
        ("a scratch connection", scratch.read(row.as_ref(), sql, &params).unwrap(), true),
        ("a common table expression", in_cte(&reader, &table, row.as_ref(), sql, &params).unwrap(), false),
      ];
      for (way, answer, same) in answers {
        assert_eq!(answer == store_answers, same, "{name}, through {way}: {answer:?} against {store_answers:?}");
      }
    }
  }

  // The reader is read only, and inside the read the core opens: a temporary
  // table lives apart from the store's file, so the store is left as it was.
  #[test]
  fn a_read_only_reader_holds_a_temporary_table_inside_its_read_and_leaves_the_store_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::create(&directory.path().join("members.db"), MEMBER).unwrap();
    store.apply(1, "INSERT INTO member VALUES ('ann', 'Ann', 'member', 1)").unwrap();
    let reader = store.reader().unwrap();
    let table = Table::of(&reader, "member").unwrap();
    let wal = || std::fs::metadata(store.path.with_extension("db-wal")).unwrap().len();
    let before = wal();
    let rebuilt = vec![text("ann"), text("Ann"), text("inviter"), Value::Integer(1)];
    reader.execute_batch("BEGIN; SELECT count(*) FROM sqlite_schema;").unwrap();
    let sql = "SELECT role FROM member WHERE id = 'ann'";
    let seen = in_temp_table(&reader, &table, Some(&rebuilt), sql, &[]).unwrap();
    reader.execute_batch("COMMIT").unwrap();
    assert_eq!(seen, vec![vec![text("inviter")]]);
    assert_eq!(query(&reader, sql, &[]).unwrap(), vec![vec![text("member")]], "the store's own row, once the temporary table is gone");
    assert_eq!(wal(), before, "nothing written to the store");
  }
}
