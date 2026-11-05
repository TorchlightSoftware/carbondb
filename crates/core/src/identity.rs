//! Identities and their devices. An identity is the hash of its first device's
//! origin. Its identity space holds that origin and the device keys it
//! authorizes; its personal space holds each device's name and tags; and the
//! system database holds the identities on this device, with each first
//! device's secret key, which no space ever holds.
//!
//! Setting up an identity, the host's work, lives here: generating the key pair,
//! creating both spaces, and keeping the identity on this device. The calls
//! that reach it are `[@carbon]`'s, in `builtins::carbon`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::error::{Result, conflict, not_found};
use crate::hash::hash;
use crate::keys;
use crate::space::{self, Origin, Ref};
use crate::store::{Side, Store};

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

// What a new identity is made from: its slug, its first device's name, and the
// services its personal space runs.
#[derive(Debug, Clone, PartialEq)]
pub struct NewIdentity {
  pub slug: String,
  pub device: String,
  pub services: Vec<String>,
}

// A device's private description, kept in the personal space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRef {
  pub pubkey: String,
  pub name: String,
  #[serde(default)]
  pub tags: Vec<String>,
}

// Set up an identity: its first device's key pair, its identity space and its
// personal space, with a ref from the first to the second, and its record in
// the system database. `op` is the hash of the call that asked for it, which
// names both spaces.
pub fn create(carbon: &Carbon, op: &str, new: &NewIdentity) -> Result<Identity> {
  let device = keys::generate()?;
  let id = hash(&json!({"key": "POST [identity]/origin", "input": {"device": device.public}}));
  let identity_space = hash(&json!({"op": op, "space": "identity"}));
  let personal = hash(&json!({"op": op, "space": "personal"}));

  let identity_origin = Origin { kind: "identity".into(), name: new.slug.clone(), services: vec!["identity".into(), "device".into()], manage: vec!["self".into()] };
  let personal_ref = Ref { space: personal.clone(), kind: "personal".into(), name: new.slug.clone(), slug: new.slug.clone(), cascade: vec![] };
  space::create(&carbon.space_path(&identity_space), &identity_origin, |store| {
    write_origin(store, &id, &device.public)?;
    add_device(store, &device.public)?;
    space::add_ref(store, &personal_ref)
  })?;

  let personal_services = [vec!["device".to_string()], new.services.iter().filter(|service| *service != "device").cloned().collect()].concat();
  let personal_origin = Origin { kind: "personal".into(), name: new.slug.clone(), services: personal_services, manage: vec![id.clone()] };
  let first_device = DeviceRef { pubkey: device.public.clone(), name: new.device.clone(), tags: vec!["manage".into()] };
  space::create(&carbon.space_path(&personal), &personal_origin, |store| put_device_ref(store, &first_device))?;

  let identity = Identity { slug: new.slug.clone(), id, space: identity_space, personal, device: device.public };
  keep(carbon.system(), &identity, &device.secret)?;
  Ok(identity)
}

// The system database: this device's identities.
pub fn open_system(path: &std::path::Path) -> Result<Store> {
  let system = Store::open(path)?;
  system.run_batch(SYSTEM_SCHEMA)?;
  Ok(system)
}

// Record an identity on this device, with its first device's secret key, in
// whatever write is open on the system database.
fn keep(system: &Store, identity: &Identity, secret: &str) -> Result<()> {
  system.run(
    Side::Writer,
    "INSERT INTO carbon_system_identity (slug, id, space, personal, device, secret) VALUES (?, ?, ?, ?, ?, ?)",
    &[json!(identity.slug), json!(identity.id), json!(identity.space), json!(identity.personal), json!(identity.device), json!(secret)],
  )?;
  Ok(())
}

pub fn find(system: &Store, slug: &str) -> Result<Identity> {
  let row = system.get(Side::Reader, "SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?;
  if row.is_null() {
    return Err(not_found(format!("No identity '{slug}' on this device")));
  }
  Ok(serde_json::from_value(row)?)
}

pub fn list(system: &Store, side: Side) -> Result<Vec<Value>> {
  system.all(side, "SELECT slug, id, space, personal, device FROM carbon_system_identity ORDER BY slug", &[])
}

// The identity's origin in its identity space, written once.
pub fn write_origin(store: &Store, id: &str, device: &str) -> Result<()> {
  if self::id(store, Side::Writer)?.is_some() {
    return Err(conflict("An identity's origin is written once"));
  }
  store.run(Side::Writer, "INSERT INTO carbon_identity (id, device) VALUES (?, ?)", &[json!(id), json!(device)])?;
  Ok(())
}

// The identity whose identity space this is.
pub fn id(store: &Store, side: Side) -> Result<Option<String>> {
  let row = store.get(side, "SELECT id FROM carbon_identity", &[])?;
  Ok(row["id"].as_str().map(str::to_string))
}

pub fn origin(store: &Store, side: Side) -> Result<Value> {
  store.get(side, "SELECT id, device FROM carbon_identity", &[])
}

// Authorize a device key in an identity space, once per key.
pub fn add_device(store: &Store, pubkey: &str) -> Result<()> {
  if !store.get(Side::Writer, "SELECT 1 AS present FROM carbon_device WHERE pubkey = ?", &[json!(pubkey)])?.is_null() {
    return Err(conflict("A device's origin is written once"));
  }
  store.run(Side::Writer, "INSERT INTO carbon_device (pubkey) VALUES (?)", &[json!(pubkey)])?;
  Ok(())
}

pub fn devices(store: &Store, side: Side) -> Result<Vec<String>> {
  let rows = store.all(side, "SELECT pubkey FROM carbon_device ORDER BY pubkey", &[])?;
  Ok(rows.iter().filter_map(|row| row["pubkey"].as_str().map(str::to_string)).collect())
}

// A second ref for the same device replaces the first.
pub fn put_device_ref(store: &Store, device: &DeviceRef) -> Result<()> {
  store.run(
    Side::Writer,
    "INSERT INTO carbon_device_ref (pubkey, name, tags) VALUES (?, ?, ?)
       ON CONFLICT (pubkey) DO UPDATE SET name = excluded.name, tags = excluded.tags",
    &[json!(device.pubkey), json!(device.name), json!(device.tags)],
  )?;
  Ok(())
}

pub fn device_refs(store: &Store, side: Side) -> Result<Vec<DeviceRef>> {
  store
    .all(side, "SELECT pubkey, name, tags FROM carbon_device_ref ORDER BY name", &[])?
    .iter()
    .map(|row| {
      Ok(DeviceRef {
        pubkey: row["pubkey"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tags: space::json_column(row, "tags")?,
      })
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{alice, carbon};

  #[test]
  fn creating_an_identity_creates_its_identity_and_personal_spaces() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(carbon.identity("alice").unwrap(), identity);

    let identity_space = carbon.open_space(&identity.space).unwrap();
    assert_eq!(id(&identity_space, Side::Reader).unwrap(), Some(identity.id.clone()));
    assert_eq!(space::managers(&identity_space, Side::Reader).unwrap(), vec![identity.id.clone()]);
    assert_eq!(devices(&identity_space, Side::Reader).unwrap(), vec![identity.device.clone()]);
    let personal_ref = space::find_ref(&identity_space, Side::Reader, "personal", "alice").unwrap().unwrap();
    assert_eq!(personal_ref.space, identity.personal);

    let personal = carbon.open_space(&identity.personal).unwrap();
    let origin = space::origin(&personal, Side::Reader).unwrap();
    assert_eq!((origin.kind.as_str(), origin.services, origin.manage), ("personal", vec!["device".to_string(), "chat".to_string()], vec![identity.id.clone()]));
    let devices = device_refs(&personal, Side::Reader).unwrap();
    assert_eq!(devices, vec![DeviceRef { pubkey: identity.device.clone(), name: "Pixel9".into(), tags: vec!["manage".into()] }]);
  }

  #[test]
  fn the_identity_id_is_the_hash_of_its_origin_and_only_the_system_database_keeps_the_secret() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(identity.id, hash(&json!({"key": "POST [identity]/origin", "input": {"device": identity.device}})));
    let row = carbon.system().get(Side::Reader, "SELECT device, secret FROM carbon_system_identity", &[]).unwrap();
    assert_eq!(row["device"], json!(identity.device));
    let secret = row["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    for id in [&identity.space, &identity.personal] {
      let bytes = std::fs::read(carbon.space_path(id)).unwrap();
      assert!(!String::from_utf8_lossy(&bytes).contains(secret));
    }
    assert!(list(carbon.system(), Side::Reader).unwrap()[0].get("secret").is_none());
  }
}
