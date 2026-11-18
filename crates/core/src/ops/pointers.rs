//! A service's pointers into its channel's log: one for each op addressed to
//! it, by hash, with what sorts it in the shared order. The op itself is read
//! from the channel's log; what became of it is the service's result. The ops
//! pending for a service are its pointers minus its results.

use serde_json::{Value, json};

use crate::Db;
use crate::error::Result;
use crate::ops::oplog::ORDER;

// Where an op addressed to a service sits in its shared order.
#[derive(Debug, Clone, PartialEq)]
pub struct Pointer {
  pub hash: String,
  pub time: u64,
  pub device: Option<String>,
}

impl Pointer {
  pub fn of(row: &Value) -> Pointer {
    Pointer { hash: row["hash"].as_str().unwrap_or_default().to_string(), time: row["time"].as_u64().unwrap_or_default(), device: row["device"].as_str().map(str::to_string) }
  }
}

// Point to an op; a pointer already there is kept.
pub fn write(db: &dyn Db, pointer: &Pointer) -> Result<()> {
  db.run("INSERT OR IGNORE INTO carbon_pointer (hash, time, device) VALUES (?, ?, ?)", &[json!(pointer.hash), json!(pointer.time), json!(pointer.device)])?;
  Ok(())
}

// A service's pointer to an op, if it has one.
pub fn get(db: &dyn Db, hash: &str) -> Result<Option<Pointer>> {
  let row = db.get("SELECT hash, time, device FROM carbon_pointer WHERE hash = ?", &[json!(hash)])?;
  Ok((!row.is_null()).then(|| Pointer::of(&row)))
}

// The first pending op, by hash: the first pointer in the shared order with no result yet.
pub fn next_pending(db: &dyn Db) -> Result<Option<String>> {
  let sql = format!("SELECT hash FROM carbon_pointer WHERE hash NOT IN (SELECT hash FROM carbon_result) {ORDER} LIMIT 1");
  Ok(db.all(&sql, &[])?.first().and_then(|row| row["hash"].as_str().map(str::to_string)))
}

// The ops this service points to, by hash, in the shared order.
pub fn hashes(db: &dyn Db) -> Result<Vec<String>> {
  Ok(db.all(&format!("SELECT hash FROM carbon_pointer {ORDER}"), &[])?.iter().filter_map(|row| row["hash"].as_str().map(str::to_string)).collect())
}
