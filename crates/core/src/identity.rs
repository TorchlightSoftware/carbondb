//! Identities on this device, as the system database keeps them. An identity
//! is the hash of its first device's `[identity]` origin; its identity space and
//! personal space hold everything else, written by ops when `[@carbon]` sets it
//! up. Only the system database holds the first device's secret key.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Result, not_found};
use crate::hash::hash;
use crate::store::{Db, Store};

const SYSTEM_SCHEMA: &str = include_str!("../sql/host/system.sql");

// An identity on this device, as anyone on it may see it: everything but the secret key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
  pub slug: String,
  pub id: String,
  pub space: String,
  pub personal: String,
  pub device: String,
}

// The id of the identity whose first device holds this public key: the hash of
// its `[identity]` origin, which every replica can recompute.
pub fn id_of(device: &str) -> String {
  hash(&json!({"key": "POST [identity]/origin", "input": {"device": device}}))
}

// The system database: this device's identities.
pub fn open_system(path: &std::path::Path) -> Result<Store> {
  let system = Store::open(path)?;
  system.run_batch(SYSTEM_SCHEMA)?;
  Ok(system)
}

// Record an identity on this device, with its first device's secret key.
pub fn keep(system: &dyn Db, identity: &Identity, secret: &str) -> Result<()> {
  system.run(
    "INSERT INTO carbon_system_identity (slug, id, space, personal, device, secret) VALUES (?, ?, ?, ?, ?, ?)",
    &[json!(identity.slug), json!(identity.id), json!(identity.space), json!(identity.personal), json!(identity.device), json!(secret)],
  )?;
  Ok(())
}

pub fn find(system: &dyn Db, slug: &str) -> Result<Identity> {
  let row = system.get("SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?;
  if row.is_null() {
    return Err(not_found(format!("No identity '{slug}' on this device")));
  }
  Ok(serde_json::from_value(row)?)
}

pub fn list(system: &dyn Db) -> Result<Vec<Value>> {
  system.all("SELECT slug, id, space, personal, device FROM carbon_system_identity ORDER BY slug", &[])
}
