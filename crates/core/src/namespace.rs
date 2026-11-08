//! Namespacing a service's SQL. Every service keeps its tables in its space's
//! one file, so each writes plain names for its tables, indexes, triggers and
//! views, and the core stores them as `service/binding/name`. Every statement
//! from every runtime arrives here before it runs, so no runtime can skip it.
//!
//! The rewrite parses each statement with sqlparser to find the names, then
//! replaces them where they stand in the original text, so the statement runs
//! exactly as written but for its names. A column qualified by a table's name,
//! as in `note.text`, follows its table. The table-valued functions SQLite
//! ships keep their names. SQLite's own tables and pragma functions describe
//! the whole file, every service in it, so a statement naming one is refused;
//! the authorizer refuses whatever else reaches outside the service's prefix.

use std::collections::BTreeSet;
use std::ops::ControlFlow;

use sqlparser::ast::{
  AlterTableOperation, ColumnOption, CreateIndex, Expr, Ident, ObjectName, ObjectNamePart, Query, RenameTableNameKind, Statement, TableConstraint, TableFactor,
  Visit, Visitor,
};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::Location;

use crate::error::{Result, ServiceError, bad_request};

// The namespace of one service's `sql` binding: `chat/db/`.
pub fn prefix(service: &str, binding: &str) -> String {
  format!("{service}/{binding}/")
}

// Refuse a service or binding name holding a slash, which separates a
// namespace's parts: a service `chat/db` with binding `db` would own
// `chat/db/db/`, inside the namespace of `chat`'s `db`.
pub fn check_parts<'a>(parts: impl IntoIterator<Item = &'a str>) -> Result<()> {
  match parts.into_iter().find(|part| part.contains('/')) {
    Some(part) => Err(bad_request(format!("'{part}' holds a slash, which would nest its namespace inside another service's, so it cannot name a service or binding"))),
    None => Ok(()),
  }
}

// SQL as SQLite should run it: its text with every name the service owns
// moved into its namespace.
pub fn rewrite(sql: &str, prefix: &str) -> Result<String> {
  let statements = Parser::parse_sql(&SQLiteDialect {}, sql).map_err(|error| unparsed(sql, error))?;
  let mut names = Names::default();
  let _ = statements.visit(&mut names);
  let renames = names.renames()?;
  Ok(splice(sql, &renames, prefix))
}

fn unparsed(sql: &str, error: impl std::fmt::Display) -> ServiceError {
  bad_request(format!("CarbonDB cannot read this SQL, so it cannot keep its names in the service's namespace: {error}: {sql}"))
}

// The names a statement mentions, sorted out as the visitor meets them.
#[derive(Default)]
struct Names {
  // Names of tables, indexes, triggers and views the service owns, where they stand.
  owned: Vec<Ident>,
  // The first part of each qualified column, as in `note` in `note.text`.
  qualifiers: Vec<Ident>,
  // Names that stand for something else in this statement: CTEs and aliases.
  local: BTreeSet<String>,
  // Names a service may not reach: another schema, as in `main.note`, SQLite's own tables, or a pragma.
  refused: Vec<String>,
}

impl Names {
  fn owns(&mut self, name: &ObjectName) {
    match name.0.as_slice() {
      [ObjectNamePart::Identifier(ident)] if whole_file(&ident.value) => self.refused.push(name.to_string()),
      [ObjectNamePart::Identifier(ident)] if !shipped_function(&ident.value) => self.owned.push(ident.clone()),
      [ObjectNamePart::Identifier(_)] => {}
      _ => self.refused.push(name.to_string()),
    }
  }

  // Every owned name to move, with each qualifier that names one of them.
  fn renames(self) -> Result<Vec<Ident>> {
    if let Some(name) = self.refused.first() {
      return Err(bad_request(format!("'{name}' reaches outside the service's own tables, which is all a service names")));
    }
    let local = self.local;
    let owned: Vec<Ident> = self.owned.into_iter().filter(|ident| !local.contains(&ident.value.to_lowercase())).collect();
    let tables: BTreeSet<String> = owned.iter().map(|ident| ident.value.to_lowercase()).collect();
    let qualifiers = self.qualifiers.into_iter().filter(|ident| tables.contains(&ident.value.to_lowercase()) && !local.contains(&ident.value.to_lowercase()));
    Ok(owned.into_iter().chain(qualifiers).collect())
  }
}

// SQLite's own tables and pragma functions, which describe every service in the file.
fn whole_file(name: &str) -> bool {
  let name = name.to_lowercase();
  name.starts_with("sqlite_") || name.starts_with("pragma_")
}

// The table-valued functions SQLite ships, which keep their names.
fn shipped_function(name: &str) -> bool {
  ["json_each", "json_tree", "generate_series"].contains(&name.to_lowercase().as_str())
}

impl Visitor for Names {
  type Break = ();

  fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
    for cte in query.with.iter().flat_map(|with| &with.cte_tables) {
      self.local.insert(cte.alias.name.value.to_lowercase());
    }
    ControlFlow::Continue(())
  }

  fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<()> {
    self.owns(relation);
    ControlFlow::Continue(())
  }

  fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<()> {
    if let TableFactor::Table { alias: Some(alias), .. } = factor {
      self.local.insert(alias.name.value.to_lowercase());
    }
    ControlFlow::Continue(())
  }

  fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
    if let Expr::CompoundIdentifier(parts) = expr
      && parts.len() > 1
    {
      self.qualifiers.push(parts[0].clone());
    }
    ControlFlow::Continue(())
  }

  // The names DDL gives or points at outside a relation: an index's or a
  // trigger's own name, what a DROP names, the table a foreign key references,
  // and a table's new name.
  fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
    match statement {
      Statement::CreateIndex(CreateIndex { name: Some(name), .. }) => self.owns(name),
      Statement::CreateTrigger(trigger) => {
        self.owns(&trigger.name);
        self.owns(&trigger.table_name);
      }
      Statement::DropTrigger(drop) => self.owns(&drop.trigger_name),
      Statement::Drop { names, .. } => names.iter().for_each(|name| self.owns(name)),
      Statement::CreateTable(table) => {
        let column_keys = table.columns.iter().flat_map(|column| &column.options).filter_map(|option| match &option.option {
          ColumnOption::ForeignKey(key) => Some(&key.foreign_table),
          _ => None,
        });
        let table_keys = table.constraints.iter().filter_map(|constraint| match constraint {
          TableConstraint::ForeignKey(key) => Some(&key.foreign_table),
          _ => None,
        });
        column_keys.chain(table_keys).collect::<Vec<_>>().into_iter().for_each(|name| self.owns(name));
      }
      Statement::AlterTable(alter) => {
        for operation in &alter.operations {
          if let AlterTableOperation::RenameTable { table_name: RenameTableNameKind::To(name) | RenameTableNameKind::As(name) } = operation {
            self.owns(name);
          }
        }
      }
      _ => {}
    }
    ControlFlow::Continue(())
  }
}

// The text with each name replaced by its namespaced, quoted form. A name met
// twice at the same place is replaced once.
fn splice(sql: &str, renames: &[Ident], prefix: &str) -> String {
  let lines = line_starts(sql);
  let mut spans: Vec<(usize, usize, String)> = renames
    .iter()
    .map(|ident| (offset(sql, &lines, ident.span.start), offset(sql, &lines, ident.span.end), format!("\"{}\"", format!("{prefix}{}", ident.value).replace('"', "\"\""))))
    .collect();
  spans.sort();
  spans.dedup_by_key(|(start, ..)| *start);
  let mut out = String::with_capacity(sql.len() + spans.len() * prefix.len());
  let mut at = 0;
  for (start, end, name) in spans {
    out.push_str(&sql[at..start]);
    out.push_str(&name);
    at = end;
  }
  out.push_str(&sql[at..]);
  out
}

fn line_starts(sql: &str) -> Vec<usize> {
  std::iter::once(0).chain(sql.match_indices('\n').map(|(index, _)| index + 1)).collect()
}

// The byte offset of a line and column, which count characters from 1.
fn offset(sql: &str, lines: &[usize], location: Location) -> usize {
  let start = lines[(location.line as usize).saturating_sub(1).min(lines.len() - 1)];
  sql[start..].char_indices().nth((location.column as usize).saturating_sub(1)).map_or(sql.len(), |(index, _)| start + index)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rewritten(sql: &str) -> String {
    rewrite(sql, "chat/db/").unwrap()
  }

  #[test]
  fn moves_every_name_a_service_owns_into_its_namespace() {
    let cases = [
      ("SELECT text FROM message WHERE id = ?", r#"SELECT text FROM "chat/db/message" WHERE id = ?"#),
      ("INSERT INTO message (id, text) VALUES (?, ?)", r#"INSERT INTO "chat/db/message" (id, text) VALUES (?, ?)"#),
      ("UPDATE message SET text = ? WHERE id = ?", r#"UPDATE "chat/db/message" SET text = ? WHERE id = ?"#),
      ("DELETE FROM message WHERE id = ?", r#"DELETE FROM "chat/db/message" WHERE id = ?"#),
      (
        "CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID",
        r#"CREATE TABLE "chat/db/message" (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID"#,
      ),
      ("CREATE TABLE IF NOT EXISTS d1_migrations (name TEXT PRIMARY KEY) WITHOUT ROWID", r#"CREATE TABLE IF NOT EXISTS "chat/db/d1_migrations" (name TEXT PRIMARY KEY) WITHOUT ROWID"#),
      ("CREATE INDEX message_by_time ON message (sent_at)", r#"CREATE INDEX "chat/db/message_by_time" ON "chat/db/message" (sent_at)"#),
      ("DROP INDEX message_by_time", r#"DROP INDEX "chat/db/message_by_time""#),
      ("DROP TABLE IF EXISTS message", r#"DROP TABLE IF EXISTS "chat/db/message""#),
      ("ALTER TABLE message ADD COLUMN edited_at INTEGER", r#"ALTER TABLE "chat/db/message" ADD COLUMN edited_at INTEGER"#),
      ("ALTER TABLE message RENAME TO post", r#"ALTER TABLE "chat/db/message" RENAME TO "chat/db/post""#),
      (
        "CREATE TABLE reply (id TEXT PRIMARY KEY, message TEXT REFERENCES message (id)) WITHOUT ROWID",
        r#"CREATE TABLE "chat/db/reply" (id TEXT PRIMARY KEY, message TEXT REFERENCES "chat/db/message" (id)) WITHOUT ROWID"#,
      ),
      (
        "CREATE TABLE reply (id TEXT PRIMARY KEY, message TEXT, FOREIGN KEY (message) REFERENCES message (id))",
        r#"CREATE TABLE "chat/db/reply" (id TEXT PRIMARY KEY, message TEXT, FOREIGN KEY (message) REFERENCES "chat/db/message" (id))"#,
      ),
      ("CREATE VIEW live AS SELECT * FROM message WHERE deleted_at IS NULL", r#"CREATE VIEW "chat/db/live" AS SELECT * FROM "chat/db/message" WHERE deleted_at IS NULL"#),
      (
        "SELECT m.text, r.id FROM message m JOIN reply AS r ON r.message = m.id",
        r#"SELECT m.text, r.id FROM "chat/db/message" m JOIN "chat/db/reply" AS r ON r.message = m.id"#,
      ),
      ("SELECT message.text FROM message", r#"SELECT "chat/db/message".text FROM "chat/db/message""#),
      (
        "WITH recent AS (SELECT * FROM message) SELECT recent.text FROM recent",
        r#"WITH recent AS (SELECT * FROM "chat/db/message") SELECT recent.text FROM recent"#,
      ),
      (
        "INSERT INTO tag (name) VALUES (?) ON CONFLICT (name) DO UPDATE SET name = excluded.name",
        r#"INSERT INTO "chat/db/tag" (name) VALUES (?) ON CONFLICT (name) DO UPDATE SET name = excluded.name"#,
      ),
      ("SELECT value FROM message, json_each(message.tags)", r#"SELECT value FROM "chat/db/message", json_each("chat/db/message".tags)"#),
      (
        "CREATE TRIGGER touch AFTER UPDATE ON message FOR EACH ROW BEGIN UPDATE message SET edited = 1 WHERE id = NEW.id; END",
        r#"CREATE TRIGGER "chat/db/touch" AFTER UPDATE ON "chat/db/message" FOR EACH ROW BEGIN UPDATE "chat/db/message" SET edited = 1 WHERE id = NEW.id; END"#,
      ),
      ("DROP TRIGGER touch", r#"DROP TRIGGER "chat/db/touch""#),
      ("INSERT INTO message (id) SELECT id FROM draft RETURNING id", r#"INSERT INTO "chat/db/message" (id) SELECT id FROM "chat/db/draft" RETURNING id"#),
      ("SELECT text FROM \"message\"", r#"SELECT text FROM "chat/db/message""#),
      (
        "SELECT id FROM message WHERE id IN (SELECT message FROM reply)",
        r#"SELECT id FROM "chat/db/message" WHERE id IN (SELECT message FROM "chat/db/reply")"#,
      ),
    ];
    for (sql, expected) in cases {
      assert_eq!(rewritten(sql), expected, "{sql}");
    }
  }

  #[test]
  fn keeps_a_default_drawn_from_the_op() {
    let sql = "CREATE TABLE note (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT) WITHOUT ROWID";
    assert_eq!(rewritten(sql), r#"CREATE TABLE "chat/db/note" (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT) WITHOUT ROWID"#);
  }

  #[test]
  fn rewrites_every_statement_of_a_migration() {
    let migration = "CREATE TABLE a (id TEXT PRIMARY KEY);\nCREATE TABLE b (id TEXT PRIMARY KEY);";
    assert_eq!(rewrite(migration, "chat/db/").unwrap(), "CREATE TABLE \"chat/db/a\" (id TEXT PRIMARY KEY);\nCREATE TABLE \"chat/db/b\" (id TEXT PRIMARY KEY);");
  }

  #[test]
  fn refuses_what_it_cannot_namespace() {
    let refused = ["SELECT * FROM main.message", "SELECT * FROM pragma_table_info('message')", "SELECT sql FROM sqlite_schema", "SELECT sql FROM SQLITE_MASTER", "SELEKT 1"];
    for sql in refused {
      assert_eq!(rewrite(sql, "chat/db/").unwrap_err().status, 400, "{sql}");
    }
  }
}
