//! `[identity]`: the identity whose identity space this is. Its origin names the
//! identity's first device, and the identity's id is derived from that device's
//! public key, so the device that signs the origin proves it founded the identity.

use std::rc::Rc;

use carbondb_service_bindings::hash::hash;
use carbondb_service_bindings::error::{Result, conflict, unknown};
use carbondb_service_bindings::{Db, HandlerContext, Package, migrations};
use serde::Deserialize;
use serde_json::{Value, json};

include!(concat!(env!("OUT_DIR"), "/package.rs"));

pub const PACKAGE: Package = Package { name: NAME, version: VERSION, binding: BINDING, init, entry };

fn init(db: &dyn Db) -> Result<()> {
  migrations::apply(db, "d1_migrations", MIGRATIONS)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  device: String,
}

// The id of the identity whose first device holds this public key: the hash of
// its origin, which every replica recomputes.
fn id_of(device: &str) -> String {
  hash(&json!({"key": "POST [identity]/origin", "input": {"device": device}}))
}

fn entry(context: Rc<dyn HandlerContext>) {
  let result = match (context.method(), context.resource()) {
    ("POST", "/origin") => serde_json::from_value(context.input().clone()).map_err(Into::into).and_then(|OriginInput { device }| write_origin(context.as_ref(), &device)),
    ("GET", "/origin") => origin(context.as_ref()),
    (method, resource) => Err(unknown(method, resource)),
  };
  let _ = context.settle(result);
}

// The identity's founding entry, written once. It answers with the identity's id.
fn write_origin(db: &dyn Db, device: &str) -> Result<Value> {
  if !origin(db)?.is_null() {
    return Err(conflict("An identity's origin is written once"));
  }
  let id = id_of(device);
  db.run("INSERT INTO identity (id, device) VALUES (?, ?)", &[json!(id), json!(device)])?;
  Ok(json!({"id": id, "device": device}))
}

fn origin(db: &dyn Db) -> Result<Value> {
  db.get("SELECT id, device FROM identity", &[])
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_identity_id_is_its_first_device_key_hashed_the_same_everywhere() {
    assert_eq!(id_of("key"), id_of("key"));
    assert_ne!(id_of("key"), id_of("other"));
  }
}
