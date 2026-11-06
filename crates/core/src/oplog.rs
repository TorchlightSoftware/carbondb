//! The op log of a space, or of the system database: what this device
//! received, each op written as it arrives, before it applies, and never
//! changed. What became of each op is its result, in `results`; the ops with
//! no result yet are pending.

use serde_json::{Value, json};

use crate::error::Result;
use crate::{Db, Op};

pub const SCHEMA: &str = include_str!("../migrations/oplog.sql");

// The shared order every replica applies ops in.
const ORDER: &str = "ORDER BY time, device, hash";

pub fn append(db: &dyn Db, op: &Op) -> Result<()> {
  db.run(
    "INSERT INTO carbon_oplog (hash, time, device, author, key, input, version) VALUES (?, ?, ?, ?, ?, ?, ?)",
    &[json!(op.hash), json!(op.time), json!(op.device), json!(op.author), json!(op.key), json!(op.input.to_string()), json!(op.version)],
  )?;
  Ok(())
}

// The log in the shared order.
pub fn entries(db: &dyn Db) -> Result<Vec<Op>> {
  db.all(&format!("SELECT hash, time, device, author, key, input, version FROM carbon_oplog {ORDER}"), &[])?.into_iter().map(to_op).collect()
}

// The first pending op: the first in the shared order with no result yet. The
// pending ops are the op log minus results, read from the store each time.
pub fn next_pending(db: &dyn Db) -> Result<Option<Op>> {
  let sql = format!("SELECT hash, time, device, author, key, input, version FROM carbon_oplog WHERE hash NOT IN (SELECT hash FROM carbon_result) {ORDER} LIMIT 1");
  db.all(&sql, &[])?.into_iter().next().map(to_op).transpose()
}

fn to_op(row: Value) -> Result<Op> {
  let text = |column: &str| row[column].as_str().map(str::to_string);
  Ok(Op::new(
    text("key").unwrap_or_default(),
    serde_json::from_str(row["input"].as_str().unwrap_or("null"))?,
    text("author"),
    text("device"),
    text("version").unwrap_or_default(),
    row["time"].as_u64().unwrap_or_default(),
    text("hash").unwrap_or_default(),
  ))
}
