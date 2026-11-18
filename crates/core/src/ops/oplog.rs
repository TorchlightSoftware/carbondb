//! A channel's log: every op this device received for the channel's
//! services, each written as it arrives, before it applies, and never changed
//! but that it gains its effects when its first run settles. Each op is
//! addressed to one service, which keeps a pointer to it (`pointers`), and may
//! name its cause: the op whose call made it, when one service called
//! another. What became of each op is its outcome here, as its service
//! recorded it; its full result stays with the service (`results`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::calls::call::{Call, Routing, address};
use crate::error::Result;
use crate::ops::effects::Made;
use crate::ops::pointers::Pointer;
use crate::{Db, Op};

// The shared order every replica applies a service's ops in. An op with no
// device, made on the system handle, sorts first among those of its time.
pub(crate) const ORDER: &str = "ORDER BY time, COALESCE(device, ''), hash";

pub(crate) const COLUMNS: &str = "hash, time, device, author, key, input, version, effects, cause";

// An op as its log row holds it, and as it travels between devices: the op;
// once its first run has settled, the effects it made, whose outputs every
// later run replays, none until then; and its cause, the op whose call made
// it, if one did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRow {
  pub op: Op,
  pub effects: Option<Vec<Made>>,
  #[serde(default)]
  pub cause: Option<String>,
}

impl LogRow {
  // An op as it arrives from this device, before it has run, made by no other op's call.
  pub fn new(op: Op) -> LogRow {
    LogRow { op, effects: None, cause: None }
  }

  // An op made by another op's call, bound to it.
  pub fn caused(op: Op, cause: Option<String>) -> LogRow {
    LogRow { op, effects: None, cause }
  }

  // The service it is addressed to, by the name its channel gives it, `@carbon` for `[@carbon]`.
  pub fn service(&self) -> Result<String> {
    Ok(address(&Call::from_key(&self.op.key)?))
  }
}

// Write a row to the log, answering whether it is new: an op already logged is a duplicate and is dropped.
pub fn append(log: &dyn Db, row: &LogRow) -> Result<bool> {
  let op = &row.op;
  let effects = row.effects.as_ref().map(serde_json::to_string).transpose()?;
  let written = log.run(
    &format!("INSERT OR IGNORE INTO carbon_oplog ({COLUMNS}, service) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
    &[json!(op.hash), json!(op.time), json!(op.device), json!(op.author), json!(op.key), json!(op.input.to_string()), json!(op.version), json!(effects), json!(row.cause), json!(row.service()?)],
  )?;
  Ok(written > 0)
}

// Take back a row this device logged for an op it then could not take.
pub fn remove(log: &dyn Db, hash: &str) -> Result<()> {
  log.run("DELETE FROM carbon_oplog WHERE hash = ?", &[json!(hash)])?;
  Ok(())
}

// An op's row, if it is logged.
pub fn row(log: &dyn Db, hash: &str) -> Result<Option<LogRow>> {
  log.all(&format!("SELECT {COLUMNS} FROM carbon_oplog WHERE hash = ?"), &[json!(hash)])?.into_iter().next().map(to_row).transpose()
}

// Write an op's effects onto its row when its first run settles; none is
// written as an empty list. A later run leaves what its first run wrote.
pub fn record_effects(log: &dyn Db, hash: &str, effects: &[Made]) -> Result<()> {
  log.run("UPDATE carbon_oplog SET effects = ? WHERE hash = ? AND effects IS NULL", &[json!(serde_json::to_string(effects)?), json!(hash)])?;
  Ok(())
}

// The effects an op's row carries once its first run has settled, in the order made.
pub fn effects(log: &dyn Db, hash: &str) -> Result<Option<Vec<Made>>> {
  let row = log.get("SELECT effects FROM carbon_oplog WHERE hash = ?", &[json!(hash)])?;
  Ok(row["effects"].as_str().map(serde_json::from_str).transpose()?)
}

// The log's rows in the shared order, as they travel between devices.
pub fn rows(log: &dyn Db) -> Result<Vec<LogRow>> {
  log.all(&format!("SELECT {COLUMNS} FROM carbon_oplog {ORDER}"), &[])?.into_iter().map(to_row).collect()
}

// The ops addressed to a service, in the shared order.
pub fn addressed(log: &dyn Db, service: &str) -> Result<Vec<Op>> {
  let rows = log.all(&format!("SELECT {COLUMNS} FROM carbon_oplog WHERE service = ? {ORDER}"), &[json!(service)])?;
  rows.into_iter().map(|row| to_row(row).map(|row| row.op)).collect()
}

// Pointers to the ops addressed to a service, for it to take up the ones it lacks.
pub fn pointers(log: &dyn Db, service: &str) -> Result<Vec<Pointer>> {
  let rows = log.all("SELECT hash, time, device FROM carbon_oplog WHERE service = ?", &[json!(service)])?;
  Ok(rows.iter().map(Pointer::of).collect())
}

// The ops a cause's call made, each with the service it is addressed to.
pub fn calls(log: &dyn Db, cause: &str) -> Result<Vec<(String, String)>> {
  let rows = log.all("SELECT hash, service FROM carbon_oplog WHERE cause = ?", &[json!(cause)])?;
  Ok(rows.iter().map(|row| (text(row, "hash"), text(row, "service"))).collect())
}

// Record what became of an op on this device, answering what it was before.
pub fn record_outcome(log: &dyn Db, hash: &str, applied: bool) -> Result<Option<bool>> {
  let before = outcome(log, hash)?;
  log.run("INSERT OR REPLACE INTO carbon_outcome (hash, outcome) VALUES (?, ?)", &[json!(hash), json!(name(applied))])?;
  Ok(before)
}

// What became of an op on this device: applied, refused, or not yet known.
pub fn outcome(log: &dyn Db, hash: &str) -> Result<Option<bool>> {
  let row = log.get("SELECT outcome FROM carbon_outcome WHERE hash = ?", &[json!(hash)])?;
  Ok(row["outcome"].as_str().map(|outcome| outcome == "applied"))
}

// Forget the outcomes of ops a rewind made pending again.
pub fn forget_outcomes(log: &dyn Db, hashes: &[String]) -> Result<()> {
  hashes.iter().try_for_each(|hash| log.run("DELETE FROM carbon_outcome WHERE hash = ?", &[json!(hash)]).map(|_| ()))
}

// The ops whose outcome here is applied, for publishing.
pub fn applied(log: &dyn Db) -> Result<BTreeSet<String>> {
  let rows = log.all("SELECT hash FROM carbon_outcome WHERE outcome = 'applied'", &[])?;
  Ok(rows.iter().map(|row| text(row, "hash")).collect())
}

fn name(applied: bool) -> &'static str {
  if applied { "applied" } else { "refused" }
}

fn text(row: &Value, column: &str) -> String {
  row[column].as_str().unwrap_or_default().to_string()
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
  let effects = row["effects"].as_str().map(serde_json::from_str).transpose()?;
  Ok(LogRow { op, effects, cause: text("cause") })
}
