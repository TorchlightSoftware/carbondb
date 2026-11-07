//! An opt-in helper for a service's `init`: apply a list of D1-style
//! migrations, each named, in order, tracking the ones applied in a table the
//! caller names, such as `d1_migrations` as Cloudflare D1 does. A service
//! ported from a Worker keeps its migrations directory and calls this from
//! `init`. The core tracks its own tables' migrations the same way.
//!
//! Schema only moves forward: a database that has applied a migration this
//! code does not declare is refused, since it was set up by newer code.

use serde_json::json;

use crate::context::Db;
use crate::error::{Result, conflict};

// A migration as its file gives it: its name, such as `0001_create_message`, and its SQL.
pub type Migration = (&'static str, &'static str);

// Apply the migrations not yet tracked in `tracking`, in order. The tracking
// table's key is the migration's name, so it never takes a key from SQLite.
pub fn apply(db: &dyn Db, tracking: &str, migrations: &[Migration]) -> Result<()> {
  db.run(&format!("CREATE TABLE IF NOT EXISTS {tracking} (name TEXT PRIMARY KEY) WITHOUT ROWID"), &[])?;
  let applied: Vec<String> = db.all(&format!("SELECT name FROM {tracking} ORDER BY name"), &[])?.iter().filter_map(|row| row["name"].as_str().map(str::to_string)).collect();
  if let Some(unknown) = applied.iter().find(|name| !migrations.iter().any(|(declared, _)| declared == name)) {
    return Err(conflict(format!("This database has applied migration '{unknown}', which this version of the service does not declare")));
  }
  for (name, sql) in migrations.iter().filter(|(name, _)| !applied.iter().any(|done| done == name)) {
    db.run(sql, &[])?;
    db.run(&format!("INSERT INTO {tracking} (name) VALUES (?)"), &[json!(name)])?;
  }
  Ok(())
}
