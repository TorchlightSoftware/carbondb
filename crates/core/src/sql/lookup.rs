//! The one row a statement reads, when it reads just one.
//!
//! SQLite's authorizer reports the tables a statement reads, never the rows,
//! so a read is of a whole table unless this module finds the one row the
//! statement looks up by its whole primary key, as in `SELECT text FROM note
//! WHERE id = ?` or `UPDATE note SET text = ? WHERE id = ?`. Then a late op
//! that writes another row of the table leaves the statement's op be. Finding
//! that row may only narrow a read, never widen what is trusted: whatever this
//! module cannot confirm reads the whole table, so a mistake here costs a
//! rewind, never a wrong state. A statement reads one row only when:
//!
//! - it is one SELECT, UPDATE or DELETE whose WHERE clause holds
//!   `column = value` terms, joined by AND, for every primary key column of
//!   the table it names, each value a literal or a parameter;
//! - SQLite's query planner plans it as one search of that table by its whole
//!   primary key and nothing else, which rules out a join, a subquery, a CTE
//!   it reads, a view, an OR, a range and a key compared under another
//!   collation;
//! - the key compares its values exactly (BINARY), so the value looked up
//!   names the one row it finds;
//! - the table has no triggers, whose reads the authorizer reports as the
//!   table's own;
//! - and, at each run, every key value bound already has the type its key
//!   column stores (text for a TEXT key, an integer for an INTEGER key), so it
//!   names the row as the changeset does.
//!
//! The authorizer confirms each run: the statement's row read is kept only
//! when the tables it read are that one table.

use std::collections::HashMap;

use rusqlite::Connection;
use serde_json::Value;
use sqlparser::ast::{BinaryOperator, Expr, FromTable, ObjectNamePart, SetExpr, Statement, TableFactor, Value as Literal};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Location, Token, Tokenizer};

use crate::store::changeset::Row;

// One row of one table, found by its whole primary key: where each key
// column's value comes from, in key order, and the type it must have.
#[derive(Debug, PartialEq)]
pub(crate) struct Lookup {
  table: String,
  key: Vec<(Source, Kind)>,
}

impl Lookup {
  // The row a run reads, from the values bound to it, when each has its key column's type.
  pub(crate) fn row(&self, params: &[Value]) -> Option<Row> {
    let values = self.key.iter().map(|(source, kind)| source.value(params).filter(|value| kind.holds(value))).collect::<Option<Vec<Value>>>()?;
    Some(Row { table: self.table.clone(), key: Value::Array(values).to_string() })
  }
}

// The row a statement looks up by its whole key, confirmed by SQLite. A
// statement sqlparser cannot read, such as SQLite's `x IN table`, runs as
// written and reads its whole tables.
pub(crate) fn find(connection: &Connection, sql: &str) -> Option<Lookup> {
  let statements = Parser::parse_sql(&SQLiteDialect {}, sql).ok()?;
  candidate(sql, &statements).and_then(|candidate| confirm(connection, sql, candidate).ok().flatten())
}

// Where a key value comes from: a parameter, by its index from 0, or a literal.
#[derive(Debug, Clone, PartialEq)]
enum Source {
  Param(usize),
  Literal(Value),
}

impl Source {
  fn value(&self, params: &[Value]) -> Option<Value> {
    match self {
      Source::Param(index) => params.get(*index).cloned(),
      Source::Literal(value) => Some(value.clone()),
    }
  }
}

// What a key column stores, by SQLite's affinity rules: text, an integer, or
// anything else, which a lookup never narrows to one row.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
  Text,
  Integer,
  Other,
}

impl Kind {
  fn of(declared: &str) -> Kind {
    let declared = declared.to_uppercase();
    match () {
      _ if declared.contains("INT") => Kind::Integer,
      _ if ["CHAR", "CLOB", "TEXT"].iter().any(|text| declared.contains(text)) => Kind::Text,
      _ => Kind::Other,
    }
  }

  // Whether a bound value is stored as it is, naming the row as the changeset does.
  fn holds(self, value: &Value) -> bool {
    match self {
      Kind::Text => value.is_string(),
      Kind::Integer => value.as_i64().is_some(),
      Kind::Other => false,
    }
  }
}

// What the text says a statement looks up: the table it reads, the name it
// goes by in the plan, each `column = value` term of its WHERE clause, and
// how many parameters it takes.
#[derive(Debug)]
struct Candidate {
  table: String,
  shown_as: String,
  terms: Vec<(String, Source)>,
  params: usize,
}

// The candidate for one row read, from a lone SELECT, UPDATE or DELETE
// whose WHERE clause holds `column = value` terms. Whether it reads anything
// more, through a join, a subquery or a CTE, is the planner's to say.
fn candidate(sql: &str, statements: &[Statement]) -> Option<Candidate> {
  let [statement] = statements else { return None };
  let (TableFactor::Table { name, alias, .. }, selection) = target(statement)? else { return None };
  let [ObjectNamePart::Identifier(table)] = name.0.as_slice() else { return None };
  let table = table.value.clone();
  let shown_as = alias.as_ref().map_or_else(|| table.clone(), |alias| alias.name.value.clone());
  let params = numbered(sql)?;
  let terms = conjuncts(selection).into_iter().filter_map(|term| equality(term, &[&table, &shown_as], &params)).collect();
  let count = params.values().map(|index| index + 1).max().unwrap_or(0);
  Some(Candidate { table, shown_as, terms, params: count })
}

// The table a statement names first, and its WHERE clause.
fn target(statement: &Statement) -> Option<(&TableFactor, &Expr)> {
  let (from, selection) = match statement {
    Statement::Query(query) => match query.body.as_ref() {
      SetExpr::Select(select) => (select.from.first()?, &select.selection),
      _ => return None,
    },
    Statement::Update(update) => (&update.table, &update.selection),
    Statement::Delete(delete) => match &delete.from {
      FromTable::WithFromKeyword(from) | FromTable::WithoutKeyword(from) => (from.first()?, &delete.selection),
    },
    _ => return None,
  };
  Some((&from.relation, selection.as_ref()?))
}

// The terms a WHERE clause joins with AND.
fn conjuncts(expr: &Expr) -> Vec<&Expr> {
  match expr {
    Expr::BinaryOp { left, op: BinaryOperator::And, right } => [conjuncts(left), conjuncts(right)].concat(),
    Expr::Nested(inner) => conjuncts(inner),
    term => vec![term],
  }
}

// A `column = value` term, either way round: the column, lower-cased, and where its value comes from.
fn equality(term: &Expr, names: &[&str], params: &HashMap<Location, usize>) -> Option<(String, Source)> {
  let Expr::BinaryOp { left, op: BinaryOperator::Eq, right } = term else { return None };
  let (column, value) = column(left, names).zip(source(right, params)).or_else(|| column(right, names).zip(source(left, params)))?;
  Some((column, value))
}

// A column of the table, plain or qualified by its name or alias.
fn column(expr: &Expr, names: &[&str]) -> Option<String> {
  match expr {
    Expr::Identifier(ident) => Some(ident.value.to_lowercase()),
    Expr::CompoundIdentifier(parts) => match parts.as_slice() {
      [table, column] if names.iter().any(|name| name.eq_ignore_ascii_case(&table.value)) => Some(column.value.to_lowercase()),
      _ => None,
    },
    _ => None,
  }
}

// A value from a parameter or a literal integer or string.
fn source(expr: &Expr, params: &HashMap<Location, usize>) -> Option<Source> {
  let Expr::Value(value) = expr else { return None };
  match &value.value {
    Literal::Placeholder(_) => params.get(&value.span.start).map(|index| Source::Param(*index)),
    Literal::Number(number, _) => number.parse::<i64>().ok().map(|number| Source::Literal(Value::from(number))),
    Literal::SingleQuotedString(text) => Some(Source::Literal(Value::from(text.clone()))),
    _ => None,
  }
}

// Each parameter's index from 0, by where it stands in the text, numbered as
// SQLite numbers them: `?` takes the next number, `?N` takes N, and a name
// takes the next number the first time it appears. The tokenizer reads `?`,
// `?N` and `$name` as placeholders, and `:name` and `@name` as a sign and a
// word, which the parser joins.
fn numbered(sql: &str) -> Option<HashMap<Location, usize>> {
  let tokens = Tokenizer::new(&SQLiteDialect {}, sql).tokenize_with_location().ok()?;
  let mut names: HashMap<String, usize> = HashMap::new();
  let mut largest = 0;
  let mut numbered = HashMap::new();
  for (at, token) in tokens.iter().enumerate() {
    let placeholder = match (&token.token, tokens.get(at + 1).map(|next| &next.token)) {
      (Token::Placeholder(placeholder), _) => placeholder.clone(),
      (Token::Colon | Token::AtSign, Some(Token::Word(word))) => format!("{}{}", token.token, word.value),
      _ => continue,
    };
    let number = match placeholder.strip_prefix('?') {
      Some("") => largest + 1,
      Some(digits) => digits.parse().ok()?,
      None => *names.entry(placeholder.clone()).or_insert(largest + 1),
    };
    largest = largest.max(number);
    numbered.insert(token.span.start, number - 1);
  }
  Some(numbered)
}

// A key column: its name, lower-cased, and the type it stores.
struct KeyColumn {
  name: String,
  kind: Kind,
}

// Confirm with SQLite that a candidate reads one row: every key column has a
// term, the key compares exactly, the table has no triggers, the parameters
// SQLite counts are the ones numbered, and the planner searches the table by
// its whole primary key.
fn confirm(connection: &Connection, sql: &str, candidate: Candidate) -> rusqlite::Result<Option<Lookup>> {
  let Some(table) = table_name(connection, &candidate.table)? else { return Ok(None) };
  let columns = key_columns(connection, &table)?;
  if columns.is_empty() {
    return Ok(None);
  }
  let key: Option<Vec<(Source, Kind)>> = columns.iter().map(|column| candidate.terms.iter().find(|(name, _)| *name == column.name).map(|(_, source)| (source.clone(), column.kind))).collect();
  let (Some(key), Some(index)) = (key, key_index(connection, &table)?) else { return Ok(None) };
  let exact = key_collations(connection, &index)?.iter().all(|collation| collation.eq_ignore_ascii_case("BINARY"));
  let shown_as = candidate.shown_as.clone();
  let searched = plan(connection, sql, candidate.params)?.is_some_and(|plan| searches_by_key(&plan, &shown_as, &index, &columns));
  Ok((exact && !has_triggers(connection, &table)? && searched).then_some(Lookup { table, key }))
}

// A table's name as its schema spells it, which names its rows in a
// changeset; SQLite matches names ignoring case. A view is no table.
fn table_name(connection: &Connection, named: &str) -> rusqlite::Result<Option<String>> {
  let mut tables = connection.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name = ? COLLATE NOCASE")?;
  let names: Vec<String> = tables.query_map([named], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
  Ok(names.into_iter().next())
}

fn key_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<KeyColumn>> {
  let mut columns = connection.prepare("SELECT name, type FROM pragma_table_xinfo(?) WHERE pk > 0 ORDER BY pk")?;
  let rows = columns.query_map([table], |row| Ok(KeyColumn { name: row.get::<_, String>(0)?.to_lowercase(), kind: Kind::of(&row.get::<_, String>(1)?) }))?;
  rows.collect()
}

// The index SQLite keeps for a table's primary key; a rowid key has none.
fn key_index(connection: &Connection, table: &str) -> rusqlite::Result<Option<String>> {
  let mut indexes = connection.prepare("SELECT name FROM pragma_index_list(?) WHERE origin = 'pk'")?;
  let names: Vec<String> = indexes.query_map([table], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
  Ok(names.into_iter().next())
}

fn key_collations(connection: &Connection, index: &str) -> rusqlite::Result<Vec<String>> {
  let mut columns = connection.prepare("SELECT coll FROM pragma_index_xinfo(?) WHERE key = 1 AND cid >= 0")?;
  let rows = columns.query_map([index], |row| row.get(0))?;
  rows.collect()
}

fn has_triggers(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
  connection.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = ?)", [table], |row| row.get(0))
}

// How SQLite's planner would run a statement, one line per step, when SQLite
// counts the parameters `numbered` found.
fn plan(connection: &Connection, sql: &str, params: usize) -> rusqlite::Result<Option<Vec<String>>> {
  let mut explained = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
  if explained.parameter_count() != params {
    return Ok(None);
  }
  // The plan never depends on the values bound, so every parameter is bound to NULL.
  let unbound = vec![rusqlite::types::Null; params];
  let steps = explained.query_map(rusqlite::params_from_iter(unbound), |row| row.get::<_, String>(3))?;
  steps.collect::<rusqlite::Result<Vec<String>>>().map(Some)
}

// Whether a plan is one search of the table by its whole primary key.
fn searches_by_key(plan: &[String], shown_as: &str, index: &str, columns: &[KeyColumn]) -> bool {
  let terms = columns.iter().map(|column| format!("{}=?", column.name)).collect::<Vec<_>>().join(" AND ");
  let searches = [format!("SEARCH {shown_as} USING PRIMARY KEY ({terms})"), format!("SEARCH {shown_as} USING INDEX {index} ({terms})"), format!("SEARCH {shown_as} USING COVERING INDEX {index} ({terms})")];
  matches!(plan, [step] if searches.iter().any(|search| search.eq_ignore_ascii_case(step)))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Db;
  use crate::call_window::CallWindow;
  use crate::sql::authorizer::Read;
  use crate::store::Store;
  use crate::store::changeset::rows;
  use serde_json::json;

  // A space holding a service's tables of every kind a lookup meets.
  fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("space.db")).unwrap();
    let schema = "
      CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID;
      CREATE TABLE pair (a TEXT, b INTEGER, text TEXT, PRIMARY KEY (a, b)) WITHOUT ROWID;
      CREATE TABLE tag (name TEXT PRIMARY KEY, color TEXT);
      CREATE TABLE counted (id INTEGER PRIMARY KEY, n INTEGER) WITHOUT ROWID;
      CREATE TABLE folded (id TEXT PRIMARY KEY COLLATE NOCASE, text TEXT) WITHOUT ROWID;
      CREATE TABLE watched (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID;
      CREATE TABLE seen (id TEXT PRIMARY KEY) WITHOUT ROWID;
      CREATE TRIGGER watched_seen AFTER UPDATE ON watched BEGIN INSERT INTO seen (id) VALUES (new.id); END;
      CREATE VIEW recent AS SELECT * FROM note;
    ";
    store.scoped(CallWindow { service: true, op: None }).run(schema, &[]).unwrap();
    (directory, store)
  }

  // A read as a test names it: `table` for the whole table, `table@key` for one row.
  fn read(named: &str) -> Read {
    match named.split_once('@') {
      Some((table, key)) => Read::Row(Row { table: table.into(), key: key.into() }),
      None => Read::Table(named.into()),
    }
  }

  // What one statement of the service reads, run in an op's transaction.
  fn reads_of(store: &Store, sql: &str, params: &[Value]) -> Vec<Read> {
    store.begin_write().unwrap();
    store.scoped(CallWindow { service: true, op: None }).all(sql, params).unwrap();
    let read = store.read_set();
    store.end_write(false).unwrap();
    read
  }

  #[test]
  fn a_statement_reads_one_row_only_when_it_looks_it_up_by_its_whole_key_and_sqlite_confirms_it() {
    let (_directory, store) = store();
    let cases: Vec<(&str, &str, Vec<Value>, Vec<&str>)> = vec![
      ("a select by its key", "SELECT text FROM note WHERE id = ?", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("a select by its key and another term", "SELECT text FROM note WHERE id = ? AND text IS NOT NULL", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("the key as a literal", "SELECT text FROM note WHERE id = 'a'", vec![], vec![r#"note@["a"]"#]),
      ("the key on the right", "SELECT text FROM note WHERE ? = id", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("an update by its key", "UPDATE note SET text = ? WHERE id = ?", vec![json!("b"), json!("a")], vec![r#"note@["a"]"#]),
      ("a delete by its key", "DELETE FROM note WHERE id = ?", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("a key of two columns", "SELECT text FROM pair WHERE b = ? AND a = ?", vec![json!(1), json!("x")], vec![r#"pair@["x",1]"#]),
      ("a table keyed by text that keeps a rowid", "SELECT color FROM tag WHERE name = ?", vec![json!("red")], vec![r#"tag@["red"]"#]),
      ("an integer key", "SELECT n FROM counted WHERE id = ?", vec![json!(5)], vec!["counted@[5]"]),
      ("numbered parameters", "SELECT text FROM note WHERE id = ?2 AND text <> ?1", vec![json!("x"), json!("a")], vec![r#"note@["a"]"#]),
      ("a named parameter", "SELECT text FROM note WHERE id = :id", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("the key after another parameter", "SELECT ? AS label, text FROM note WHERE id = ?", vec![json!("label"), json!("a")], vec![r#"note@["a"]"#]),
      ("the table named in another case", "SELECT text FROM NOTE WHERE ID = ?", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("a qualified column and an alias", "SELECT n.text FROM note AS n WHERE n.id = ?", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("an insert, which reads nothing", "INSERT INTO note (id, text) VALUES (?, ?)", vec![json!("z"), json!("new")], vec![]),
      ("half of a key of two columns", "SELECT text FROM pair WHERE a = ?", vec![json!("x")], vec!["pair"]),
      ("an integer key looked up with text", "SELECT n FROM counted WHERE id = ?", vec![json!("5")], vec!["counted"]),
      ("a text key looked up with an integer", "SELECT text FROM note WHERE id = ?", vec![json!(5)], vec!["note"]),
      ("another column", "SELECT text FROM note WHERE text = ?", vec![json!("a")], vec!["note"]),
      ("no WHERE clause", "SELECT text FROM note", vec![], vec!["note"]),
      ("an OR", "SELECT text FROM note WHERE id = ? OR id = ?", vec![json!("a"), json!("b")], vec!["note"]),
      ("a range", "SELECT text FROM note WHERE id > ?", vec![json!("a")], vec!["note"]),
      ("an IN list", "SELECT text FROM note WHERE id IN (?)", vec![json!("a")], vec!["note"]),
      ("a join", "SELECT n.text FROM note n JOIN tag t ON t.name = n.text WHERE n.id = ?", vec![json!("a")], vec!["note", "tag"]),
      ("a subquery of the same table", "SELECT text FROM note WHERE id = ? AND text IN (SELECT text FROM note)", vec![json!("a")], vec!["note"]),
      ("a subquery in what it selects", "SELECT (SELECT count(*) FROM note) AS n FROM note WHERE id = ?", vec![json!("a")], vec!["note"]),
      ("a CTE it never reads", "WITH one AS (SELECT 1) SELECT text FROM note WHERE id = ?", vec![json!("a")], vec![r#"note@["a"]"#]),
      ("a CTE it reads", "WITH texts AS (SELECT text FROM note) SELECT text FROM note WHERE id = ? AND text IN (SELECT text FROM texts)", vec![json!("a")], vec!["note"]),
      ("a second table, without JOIN", "SELECT n.text FROM note n, tag t WHERE n.id = ? AND t.name = ?", vec![json!("a"), json!("red")], vec!["note", "tag"]),
      ("a subquery in the FROM clause", "SELECT text FROM (SELECT * FROM note) WHERE id = ?", vec![json!("a")], vec!["note"]),
      ("a view", "SELECT text FROM recent WHERE id = ?", vec![json!("a")], vec!["note", "recent"]),
      ("a key that ignores case", "SELECT text FROM folded WHERE id = ?", vec![json!("a")], vec!["folded"]),
      ("the key compared ignoring case", "SELECT text FROM note WHERE id = ? COLLATE NOCASE", vec![json!("a")], vec!["note"]),
      ("a table with a trigger", "SELECT text FROM watched WHERE id = ?", vec![json!("a")], vec!["watched"]),
    ];
    for (name, sql, params, expected) in cases {
      assert_eq!(reads_of(&store, sql, &params), expected.into_iter().map(read).collect::<Vec<_>>(), "{name}");
    }
  }

  #[test]
  fn several_statements_run_at_once_read_their_whole_tables() {
    let (_directory, store) = store();
    store.begin_write().unwrap();
    store.scoped(CallWindow { service: true, op: None }).run("SELECT text FROM note WHERE id = 'a'; SELECT text FROM note WHERE id = 'b'", &[]).unwrap();
    assert_eq!(store.read_set(), vec![read("note")]);
    store.end_write(false).unwrap();
  }

  #[test]
  fn a_row_read_names_its_row_as_the_changeset_does() {
    let (_directory, store) = store();
    let scoped = store.scoped(CallWindow { service: true, op: None });
    scoped.run("INSERT INTO pair (a, b, text) VALUES ('x', 1, 'old')", &[]).unwrap();
    store.begin_write().unwrap();
    scoped.run("UPDATE pair SET text = ? WHERE a = ? AND b = ?", &[json!("new"), json!("x"), json!(1)]).unwrap();
    let written = rows(&store.changeset().unwrap()).unwrap();
    assert_eq!(store.read_set(), written.into_iter().map(Read::Row).collect::<Vec<_>>());
    store.end_write(true).unwrap();
  }

  #[test]
  fn sqlite_plans_a_lookup_by_key_as_one_search_of_its_key() {
    let connection = crate::sql::tests::connection();
    let table = "note";
    let index = key_index(&connection, table).unwrap();
    let plan = plan(&connection, "SELECT text FROM note WHERE id = ?", 1).unwrap();
    assert_eq!((index.clone(), plan), (Some("sqlite_autoindex_note_1".into()), Some(vec!["SEARCH note USING PRIMARY KEY (id=?)".to_string()])));
    assert_eq!(key_collations(&connection, &index.unwrap()).unwrap(), vec!["BINARY".to_string()]);
  }
}
