//! `[identity]`: the identity whose identity space this is. Its origin names the
//! first device's public key, and its id is that origin's hash.

use std::rc::Rc;

use carbondb_service_api::error::{Result, conflict};
use carbondb_service_api::hash::hash;
use carbondb_service_api::{Db, Invocation, Package, answer};
use serde::Deserialize;
use serde_json::{Value, json};

include!(concat!(env!("OUT_DIR"), "/package.rs"));

pub const PACKAGE: Package = Package { name: NAME, migrations: MIGRATIONS, entry };

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  device: String,
}

// The id of the identity whose first device holds this public key: the hash of
// its origin, which every replica recomputes.
pub fn id_of(device: &str) -> String {
  hash(&json!({"key": "POST [identity]/origin", "input": {"device": device}}))
}

fn entry(invocation: Rc<dyn Invocation>) {
  answer(invocation, handle)
}

fn handle(invocation: &Rc<dyn Invocation>) -> Result<Value> {
  match (invocation.call().method(), invocation.call().resource.as_str()) {
    ("POST", "/origin") => {
      let OriginInput { device } = serde_json::from_value(invocation.input().clone())?;
      write_origin(invocation.as_ref(), &device)
    }
    ("GET", "/origin") => origin(invocation.as_ref()),
    _ => Err(invocation.call().unknown()),
  }
}

// The identity's founding entry, written once.
fn write_origin(db: &dyn Db, device: &str) -> Result<Value> {
  if !origin(db)?.is_null() {
    return Err(conflict("An identity's origin is written once"));
  }
  let id = id_of(device);
  db.run("INSERT INTO carbon_identity (id, device) VALUES (?, ?)", &[json!(id), json!(device)])?;
  Ok(json!({"id": id, "device": device}))
}

fn origin(db: &dyn Db) -> Result<Value> {
  db.get("SELECT id, device FROM carbon_identity", &[])
}
