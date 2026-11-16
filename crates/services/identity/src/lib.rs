//! `[identity]`: the identity whose channel this is, and the devices that act
//! for it. Its origin names the identity's first device, and the identity's id
//! is derived from that device's public key, so the device that signs the
//! origin proves it founded the identity. It runs only in the identity's own
//! channel, shared by the person's own devices and never with anyone else.
//!
//! Adding a device needs a handshake with it, which is an effect; single-node
//! it cannot be reached. Renaming or retagging one needs the identity itself.

use std::rc::Rc;

use carbondb_service_bindings::error::{Result, conflict, forbidden, not_found, unknown};
use carbondb_service_bindings::hash::hash;
use carbondb_service_bindings::{Db, HandlerContext, Package, Reply, migrations};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PACKAGE: Package = Package { setup, entry };

#[cfg(target_arch = "wasm32")]
carbondb_service_bindings::serve!(PACKAGE);

// Its tables, set up by `setup` on every open.
const MIGRATIONS: &[migrations::Migration] = &[("0001_identity", include_str!("../migrations/0001_identity.sql"))];

fn setup(db: &dyn Db) -> Result<()> {
  migrations::apply(db, "d1_migrations", MIGRATIONS)
}

// The founding entry: the first device, with its name and tags.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  device: String,
  name: String,
  #[serde(default)]
  tags: Vec<String>,
}

// A device's name and tags.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Naming {
  name: String,
  #[serde(default)]
  tags: Vec<String>,
}

// A device acting for the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
  pub pubkey: String,
  pub name: String,
  pub tags: Vec<String>,
}

// The id of the identity whose first device holds this public key: the hash
// of its origin, which every replica, and the device founding it, recomputes.
pub fn id_of(device: &str) -> String {
  hash(&json!({"key": "POST [identity]/origin", "input": {"device": device}}))
}

type Context = Rc<dyn HandlerContext>;

fn entry(context: Context) {
  let resource = context.resource().to_string();
  let result = match (context.method(), resource.as_str()) {
    ("POST", "/origin") => serde_json::from_value(context.input().clone()).map_err(Into::into).and_then(|origin| write_origin(context.as_ref(), origin)),
    ("GET", "/origin") => origin(context.as_ref()),
    ("GET", "/devices") => devices(context.as_ref()).map(|devices| json!(devices)),
    ("POST", "/devices") => return enroll(context),
    ("PUT", resource) if resource.starts_with("/devices/") => rename(context.as_ref(), &resource["/devices/".len()..]),
    (method, resource) => Err(unknown(method, resource)),
  };
  let _ = context.settle(result);
}

// The identity's founding entry, written once, with its first device. It answers with the identity's id.
fn write_origin(db: &dyn Db, origin: OriginInput) -> Result<Value> {
  if !origin_row(db)?.is_null() {
    return Err(conflict("An identity's origin is written once"));
  }
  let id = id_of(&origin.device);
  db.run("INSERT INTO identity (id, device) VALUES (?, ?)", &[json!(id), json!(origin.device)])?;
  db.run("INSERT INTO device (pubkey, name, tags) VALUES (?, ?, ?)", &[json!(origin.device), json!(origin.name), json!(origin.tags)])?;
  Ok(json!({"id": id, "device": origin.device}))
}

fn origin_row(db: &dyn Db) -> Result<Value> {
  db.get("SELECT id, device FROM identity", &[])
}

fn origin(db: &dyn Db) -> Result<Value> {
  let row = origin_row(db)?;
  match row.is_null() {
    true => Err(not_found("This identity has no origin yet")),
    false => Ok(row),
  }
}

// Only the identity itself changes its devices.
fn as_itself(context: &dyn HandlerContext) -> Result<()> {
  let id = origin(context)?["id"].clone();
  match context.actor().identity.as_deref() == id.as_str() {
    true => Ok(()),
    false => Err(forbidden("Only the identity itself changes its devices")),
  }
}

// Add a device: a handshake with the new device, as an effect, which answers
// its key once it has accepted; or, without a code, a code for it to scan.
fn enroll(context: Context) {
  if let Err(error) = as_itself(context.as_ref()) {
    let _ = context.settle(Err(error));
    return;
  }
  let caller = Rc::clone(&context);
  let reply: Reply = Box::new(move |enrolled| {
    let added = enrolled.and_then(|enrolled| match enrolled["pubkey"].as_str() {
      Some(pubkey) => {
        let naming: Naming = serde_json::from_value(json!({"name": caller.input()["name"], "tags": caller.input()["tags"].as_array().cloned().unwrap_or_default()}))?;
        caller.run("INSERT OR REPLACE INTO device (pubkey, name, tags) VALUES (?, ?, ?)", &[json!(pubkey), json!(naming.name), json!(naming.tags)])?;
        Ok(json!({"pubkey": pubkey, "name": naming.name, "tags": naming.tags}))
      }
      None => Ok(enrolled),
    });
    let _ = caller.settle(added);
  });
  let input = context.input().clone();
  Rc::clone(&context).effect("enroll", input, reply)
}

fn rename(context: &dyn HandlerContext, pubkey: &str) -> Result<Value> {
  as_itself(context)?;
  let Naming { name, tags } = serde_json::from_value(context.input().clone())?;
  if context.run("UPDATE device SET name = ?, tags = ? WHERE pubkey = ?", &[json!(name), json!(tags), json!(pubkey)])? == 0 {
    return Err(not_found(format!("No device {pubkey} acts for this identity")));
  }
  Ok(json!({"pubkey": pubkey, "name": name, "tags": tags}))
}

fn devices(db: &dyn Db) -> Result<Vec<Device>> {
  db.all("SELECT pubkey, name, tags FROM device ORDER BY name", &[])?
    .iter()
    .map(|row| {
      Ok(Device {
        pubkey: row["pubkey"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tags: serde_json::from_str(row["tags"].as_str().unwrap_or("[]"))?,
      })
    })
    .collect()
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
