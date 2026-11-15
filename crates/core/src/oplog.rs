//! The op log of a space, or of the system database: what this device
//! received, each op written as it arrives, before it applies, and never
//! changed, but that a local op gains its sub-calls when it first commits.
//! What became of each op is its result, in `results`; the ops with no result
//! yet are pending.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::Result;
use crate::{Db, Op};

// The shared order every replica applies ops in. An op with no device, made
// on the system handle, sorts first among those of its time.
pub(crate) const ORDER: &str = "ORDER BY time, COALESCE(device, ''), hash";

const COLUMNS: &str = "hash, time, device, author, key, input, version, sub_calls";

// An op as its log row holds it, and as it travels between devices: the op
// and, for a local op that has run, the sub-calls it made, which every replay
// runs in place of the local op. None until a local op first commits; every
// other op has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRow {
  pub op: Op,
  pub sub_calls: Option<Vec<Op>>,
}

impl LogRow {
  // An op as it arrives from this device, before it has run.
  pub fn new(op: Op) -> LogRow {
    LogRow { op, sub_calls: None }
  }
}

// Write a row to the log. An op already logged is a duplicate and is dropped.
pub fn append(db: &dyn Db, row: &LogRow) -> Result<()> {
  let op = &row.op;
  let sub_calls = row.sub_calls.as_ref().map(serde_json::to_string).transpose()?;
  db.run(
    &format!("INSERT OR IGNORE INTO carbon_oplog ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"),
    &[json!(op.hash), json!(op.time), json!(op.device), json!(op.author), json!(op.key), json!(op.input.to_string()), json!(op.version), json!(sub_calls)],
  )?;
  Ok(())
}

// Write a local op's sub-calls onto its row when it first commits, in the
// transaction that records its result; none is written as an empty list. A
// replay of the op leaves what its first run wrote.
pub fn record_sub_calls(db: &dyn Db, hash: &str, sub_calls: &[Op]) -> Result<()> {
  db.run("UPDATE carbon_oplog SET sub_calls = ? WHERE hash = ? AND sub_calls IS NULL", &[json!(serde_json::to_string(sub_calls)?), json!(hash)])?;
  Ok(())
}

// The sub-calls a local op's row carries once it has run, in the order it made them.
pub fn sub_calls(db: &dyn Db, hash: &str) -> Result<Option<Vec<Op>>> {
  let row = db.get("SELECT sub_calls FROM carbon_oplog WHERE hash = ?", &[json!(hash)])?;
  Ok(row["sub_calls"].as_str().map(serde_json::from_str).transpose()?)
}

// The log in the shared order.
pub fn entries(db: &dyn Db) -> Result<Vec<Op>> {
  Ok(rows(db)?.into_iter().map(|row| row.op).collect())
}

// The log's rows in the shared order, as they travel between devices.
pub fn rows(db: &dyn Db) -> Result<Vec<LogRow>> {
  db.all(&format!("SELECT {COLUMNS} FROM carbon_oplog {ORDER}"), &[])?.into_iter().map(to_row).collect()
}

// The first pending op: the first in the shared order with no result yet. The
// pending ops are the op log minus results, read from the store each time.
pub fn next_pending(db: &dyn Db) -> Result<Option<LogRow>> {
  let sql = format!("SELECT {COLUMNS} FROM carbon_oplog WHERE hash NOT IN (SELECT hash FROM carbon_result) {ORDER} LIMIT 1");
  db.all(&sql, &[])?.into_iter().next().map(to_row).transpose()
}

pub(crate) fn to_row(row: Value) -> Result<LogRow> {
  let text = |column: &str| row[column].as_str().map(str::to_string);
  let op = Op::new(
    text("key").unwrap_or_default(),
    serde_json::from_str(row["input"].as_str().unwrap_or("null"))?,
    text("author"),
    text("device"),
    text("version").unwrap_or_default(),
    row["time"].as_u64().unwrap_or_default(),
    text("hash").unwrap_or_default(),
  );
  let sub_calls = row["sub_calls"].as_str().map(serde_json::from_str).transpose()?;
  Ok(LogRow { op, sub_calls })
}
