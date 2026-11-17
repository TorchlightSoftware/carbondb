//! What this device published of its own ops, by op hash (`carbon_published`):
//! the op log is the outbox, and the ops to publish are this device's own
//! applied ops not yet recorded here. A rewind can make an op publishable late
//! and out of log order, so a position in the log would not do; a rewind
//! forgets results, but leaves this alone.

use serde_json::json;

use crate::Db;
use crate::error::Result;
use crate::ops::oplog::{self, COLUMNS, LogRow, ORDER};

// This device's applied ops not yet published, in the shared order.
pub fn unpublished(db: &dyn Db, device: &str) -> Result<Vec<LogRow>> {
  let sql = format!(
    "SELECT {COLUMNS} FROM carbon_oplog WHERE device = ? AND hash IN (SELECT hash FROM carbon_result WHERE outcome = 'applied') AND hash NOT IN (SELECT hash FROM carbon_published) {ORDER}"
  );
  db.all(&sql, &[json!(device)])?.into_iter().map(oplog::to_row).collect()
}

// Record that p2panda accepted an op for publishing.
pub fn record(db: &dyn Db, hash: &str) -> Result<()> {
  db.run("INSERT OR IGNORE INTO carbon_published (hash) VALUES (?)", &[json!(hash)])?;
  Ok(())
}
