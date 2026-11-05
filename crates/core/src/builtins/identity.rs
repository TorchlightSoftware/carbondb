//! `[identity]`: the identity whose identity space this is. Its origin names the
//! first device's public key, and its id is that origin's hash.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Result, conflict};
use crate::identity::id_of;
use crate::invocation::Invocation;
use crate::store::Db;

pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/builtins/identity/001.sql")];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  device: String,
}

pub fn handle(invocation: &Invocation) -> Result<Value> {
  match (invocation.call.method(), invocation.call.resource.as_str()) {
    ("POST", "/origin") => {
      let OriginInput { device } = serde_json::from_value(invocation.input.clone())?;
      write_origin(invocation, &device)
    }
    ("GET", "/origin") => origin(invocation),
    _ => Err(invocation.call.unknown()),
  }
}

// The identity's founding entry, written once.
fn write_origin(db: &dyn Db, device: &str) -> Result<Value> {
  if id(db)?.is_some() {
    return Err(conflict("An identity's origin is written once"));
  }
  let id = id_of(device);
  db.run("INSERT INTO carbon_identity (id, device) VALUES (?, ?)", &[json!(id), json!(device)])?;
  Ok(json!({"id": id, "device": device}))
}

// The identity whose identity space this is.
pub fn id(db: &dyn Db) -> Result<Option<String>> {
  let row = db.get("SELECT id FROM carbon_identity", &[])?;
  Ok(row["id"].as_str().map(str::to_string))
}

fn origin(db: &dyn Db) -> Result<Value> {
  db.get("SELECT id, device FROM carbon_identity", &[])
}
