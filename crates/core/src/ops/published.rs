//! What this device published of its own ops, by op hash (`carbon_published`,
//! in the channel's log): the log is the outbox. An op is publishable once its
//! outcome here is applied and so is its cause's, all the way up, so a
//! cause's calls go out only once the op that made them succeeded. A rewind
//! can make an op publishable late and out of log order, so a position in the
//! log would not do; a rewind forgets outcomes, but leaves this alone.

use std::collections::BTreeSet;

use serde_json::json;

use crate::Db;
use crate::error::Result;
use crate::ops::oplog::{self, COLUMNS, LogRow, ORDER};

// This device's ops in a channel not yet published that are publishable, in the shared order.
pub fn unpublished(log: &dyn Db, device: &str) -> Result<Vec<LogRow>> {
  let sql = format!("SELECT {COLUMNS} FROM carbon_oplog WHERE device = ? AND hash NOT IN (SELECT hash FROM carbon_published) {ORDER}");
  let rows = log.all(&sql, &[json!(device)])?.into_iter().map(oplog::to_row).collect::<Result<Vec<LogRow>>>()?;
  let applied = oplog::applied(log)?;
  Ok(rows.into_iter().filter(|row| succeeded(log, &applied, row)).collect())
}

// Whether an op applied here, and so did each cause above it.
fn succeeded(log: &dyn Db, applied: &BTreeSet<String>, row: &LogRow) -> bool {
  let mut cause = Some(row.op.hash.clone());
  while let Some(hash) = cause {
    if !applied.contains(&hash) {
      return false;
    }
    cause = oplog::row(log, &hash).ok().flatten().and_then(|row| row.cause);
  }
  true
}

// Record that p2panda accepted an op for publishing.
pub fn record(log: &dyn Db, hash: &str) -> Result<()> {
  log.run("INSERT OR IGNORE INTO carbon_published (hash) VALUES (?)", &[json!(hash)])?;
  Ok(())
}
