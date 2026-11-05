//! The op log of a space, or of the system database: each op applied there,
//! appended once in the transaction that applied it, so the row commits with
//! the op's writes or not at all. A row also keeps what the op returned and the
//! calls it made, this device's own record; a replica recomputes both.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Result;
use crate::op::Op;
use crate::store::Db;

pub const SCHEMA: &str = include_str!("../sql/host/oplog.sql");

// An op as the log holds it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
  pub hash: String,
  pub time: u64,
  pub device: Option<String>,
  pub author: Option<String>,
  pub key: String,
  pub input: Value,
  pub version: usize,
  pub output: Value,
  pub calls: Value,
}

pub fn append(db: &dyn Db, op: &Op, output: &Value, calls: &Value) -> Result<()> {
  db.run(
    "INSERT INTO carbon_oplog (hash, time, device, author, key, input, version, output, calls) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    &[json!(op.hash), json!(op.time), json!(op.device), json!(op.author), json!(op.key), json(&op.input), json!(op.version), json(output), json(calls)],
  )?;
  Ok(())
}

// The log in the order ops were applied.
pub fn entries(db: &dyn Db) -> Result<Vec<Entry>> {
  db.all("SELECT hash, time, device, author, key, input, version, output, calls FROM carbon_oplog ORDER BY time, device, hash", &[])?
    .into_iter()
    .map(|mut row| {
      for column in ["input", "output", "calls"] {
        row[column] = serde_json::from_str(row[column].as_str().unwrap_or("null"))?;
      }
      Ok(serde_json::from_value(row)?)
    })
    .collect()
}

// A value as JSON text, so every value, strings included, reads back as itself.
fn json(value: &Value) -> Value {
  Value::String(value.to_string())
}
