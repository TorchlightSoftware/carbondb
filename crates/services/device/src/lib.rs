//! `[device]`: in an identity space, the device keys it authorizes (origins);
//! in a personal space, each device's name and tags (refs). Writing either
//! needs manage on the space, which `[space]` answers.

use std::rc::Rc;

use carbondb_service_api::error::{Result, bad_request, forbidden, single_node};
use carbondb_service_api::{Db, Invocation, Package, answer, ask};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

include!(concat!(env!("OUT_DIR"), "/package.rs"));

pub const PACKAGE: Package = Package { name: NAME, migrations: MIGRATIONS, entry };

// A device's private description, kept in the personal space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRef {
  pub pubkey: String,
  pub name: String,
  #[serde(default)]
  pub tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  pubkey: String,
}

fn entry(invocation: Rc<dyn Invocation>) {
  answer(invocation, handle)
}

fn handle(invocation: &Rc<dyn Invocation>) -> Result<Value> {
  let input = || invocation.input().clone();
  match (invocation.call().method(), invocation.call().resource.as_str()) {
    ("POST", "/origin") => {
      require_manage(invocation)?;
      if ask(invocation, "GET [space]/origin", Value::Null)?["kind"] != json!("identity") {
        return Err(bad_request("A device's origin is written in its identity space"));
      }
      let OriginInput { pubkey } = serde_json::from_value(input())?;
      add_device(invocation.as_ref(), &pubkey)
    }
    ("GET", "/devices") => Ok(json!(devices(invocation.as_ref())?)),
    ("POST", "/ref") => {
      require_manage(invocation)?;
      let device: DeviceRef = serde_json::from_value(input())?;
      put_device_ref(invocation.as_ref(), &device)?;
      Ok(json!(device))
    }
    ("GET", "/refs") => Ok(json!(device_refs(invocation.as_ref())?)),
    _ => Err(invocation.call().unknown()),
  }
}

fn require_manage(invocation: &Rc<dyn Invocation>) -> Result<()> {
  let managers = ask(invocation, "GET [space]/manage", Value::Null)?;
  let actor = invocation.actor().identity.clone().ok_or_else(|| forbidden("Managing a space needs an acting identity"))?;
  if managers.as_array().is_some_and(|managers| managers.contains(&json!(actor))) {
    Ok(())
  } else {
    Err(forbidden("Only a manager of this space may change its devices"))
  }
}

// Authorize the first device's key. A second device is authorized by an
// existing one over the network, which single-node cannot do.
fn add_device(db: &dyn Db, pubkey: &str) -> Result<Value> {
  if !devices(db)?.is_empty() {
    return Err(single_node("Adding a device"));
  }
  db.run("INSERT INTO carbon_device (pubkey) VALUES (?)", &[json!(pubkey)])?;
  Ok(json!({"pubkey": pubkey}))
}

fn devices(db: &dyn Db) -> Result<Vec<String>> {
  let rows = db.all("SELECT pubkey FROM carbon_device ORDER BY pubkey", &[])?;
  Ok(rows.iter().filter_map(|row| row["pubkey"].as_str().map(str::to_string)).collect())
}

// A second ref for the same device replaces the first.
fn put_device_ref(db: &dyn Db, device: &DeviceRef) -> Result<()> {
  db.run(
    "INSERT INTO carbon_device_ref (pubkey, name, tags) VALUES (?, ?, ?)
       ON CONFLICT (pubkey) DO UPDATE SET name = excluded.name, tags = excluded.tags",
    &[json!(device.pubkey), json!(device.name), json!(device.tags)],
  )?;
  Ok(())
}

fn device_refs(db: &dyn Db) -> Result<Vec<DeviceRef>> {
  db.all("SELECT pubkey, name, tags FROM carbon_device_ref ORDER BY name", &[])?
    .iter()
    .map(|row| {
      Ok(DeviceRef {
        pubkey: row["pubkey"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tags: serde_json::from_str(row["tags"].as_str().unwrap_or("[]"))?,
      })
    })
    .collect()
}
