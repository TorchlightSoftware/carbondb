//! Messages that failed the check every received op passes (`carbon_error`,
//! in the channel's log), kept as evidence and never applied: keyed by their
//! p2panda operation hash, since the op hash a message claims is unproven,
//! with the signed operation, its p2panda author, when it arrived and which
//! check it failed.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Db;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
  pub operation: String,
  pub author: String,
  pub arrived: u64,
  pub failed: String,
  pub signed: String,
}

pub fn record(log: &dyn Db, evidence: &Evidence) -> Result<()> {
  log.run(
    "INSERT OR IGNORE INTO carbon_error (operation, author, arrived, failed, signed) VALUES (?, ?, ?, ?, ?)",
    &[json!(evidence.operation), json!(evidence.author), json!(evidence.arrived), json!(evidence.failed), json!(evidence.signed)],
  )?;
  Ok(())
}

// The evidence kept here, oldest first.
pub fn all(log: &dyn Db) -> Result<Vec<Evidence>> {
  log.all("SELECT operation, author, arrived, failed, signed FROM carbon_error ORDER BY arrived, operation", &[])?.into_iter().map(|row| Ok(serde_json::from_value(row)?)).collect()
}
