//! `[@carbon]`, the core's local service, and the system database it keeps.
//!
//! The system database is this device's root: the identities on it, with their
//! device keys. Every other space is a file beside it, named by space id.
//! `Carbon` opens spaces and routes `[@carbon]` calls; what a new identity or
//! space is made of lives in `create`.
//!
//! This is the single-node core: no p2panda, no network and no op log. Calls
//! that need peers are refused.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::builtins;
use crate::call::{Actor, Call, Outcome};
use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::create;
use crate::error::{Result, bad_request, not_built, not_found, single_node};
use crate::hash::hash;
use crate::space;
use crate::store::{Side, Store};

const SYSTEM_SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS carbon_system_identity (
    slug TEXT PRIMARY KEY, id TEXT NOT NULL UNIQUE, space TEXT NOT NULL, personal TEXT NOT NULL,
    device TEXT NOT NULL, secret TEXT NOT NULL
  );
";

// An identity on this device, as anyone on it may see it: everything but the secret key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
  pub slug: String,
  pub id: String,
  pub space: String,
  pub personal: String,
  pub device: String,
}

// When an op happened, and its hash over the envelope and that time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stamp {
  pub time: u64,
  pub hash: String,
}

pub struct Carbon {
  config: Config,
  system: Store,
  clock: Clock,
}

impl Carbon {
  pub fn open(config_path: &Path) -> Result<Carbon> {
    let config = Config::load(config_path)?;
    if config.sync != SyncMode::None {
      return Err(not_built("Replicated mode"));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    let system = Store::open(&config.system)?;
    system.run_batch(SYSTEM_SCHEMA)?;
    Ok(Carbon { config, system, clock: Clock::default() })
  }

  // Stamp an op's envelope with a time, and hash the two together.
  pub fn stamp(&self, envelope: &Value) -> Stamp {
    let time = self.clock.now();
    Stamp { time, hash: hash(&json!({"envelope": envelope, "time": time})) }
  }

  // Where a space's database sits: beside the system database, named by space id.
  pub(crate) fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  pub fn open_space(&self, id: &str) -> Result<Store> {
    let path = self.space_path(id);
    if !path.exists() {
      return Err(not_found(format!("No space {id} on this device")));
    }
    Store::open(&path)
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    let row = self.system.get(Side::Reader, "SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?;
    if row.is_null() {
      return Err(not_found(format!("No identity '{slug}' on this device")));
    }
    Ok(serde_json::from_value(row)?)
  }

  // A call on the system handle.
  pub fn call_system(&self, call: &Call, input: Value) -> Result<Outcome> {
    match (call.confined, call.service.as_str(), call.method(), call.resource.as_str()) {
      (true, "carbon", "POST", "/identity") => {
        let (identity, secret) = create::identity(self, serde_json::from_value(input)?)?;
        self.keep_identity(&identity, &secret)
      }
      (true, "carbon", "GET", "/identities") => Outcome::read(&self.system, |system| {
        Ok(json!(system.all(Side::Reader, "SELECT slug, id, space, personal, device FROM carbon_system_identity ORDER BY slug", &[])?))
      }),
      (true, "carbon", "POST", "/devices") => Err(single_node("Adding a device")),
      _ => Err(call.unknown()),
    }
  }

  // A call on a space's handle, to `[@carbon]` or a replicated builtin.
  pub fn call_space(&self, store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
    if !call.confined {
      return builtins::call(store, actor, call, input);
    }
    match (call.service.as_str(), call.method(), call.resource.as_str()) {
      ("carbon", "GET", "/status") => Outcome::read(store, |store| status(store, actor)),
      ("carbon", "POST", "/space") => create::nested_space(self, store, actor, call, input),
      ("carbon", "GET", "/manage") => Outcome::read(store, |store| Ok(json!(space::managers(store, Side::Reader)?))),
      ("carbon", _, "/members" | "/join" | "/sync/start" | "/sync/stop") => Err(single_node(&call.key())),
      ("carbon", "POST", "/manage") => Err(not_built("Changing a manage list")),
      ("carbon", "DELETE", "/data") => Err(not_built("Deleting a space's data")),
      _ => Err(call.unknown()),
    }
  }

  // Record a new identity, with its device's secret key, in the system database.
  fn keep_identity(&self, identity: &Identity, secret: &str) -> Result<Outcome> {
    Outcome::write(&self.system, |system| {
      system.run(
        Side::Writer,
        "INSERT INTO carbon_system_identity (slug, id, space, personal, device, secret) VALUES (?, ?, ?, ?, ?, ?)",
        &[json!(identity.slug), json!(identity.id), json!(identity.space), json!(identity.personal), json!(identity.device), json!(secret)],
      )?;
      Ok(json!(identity))
    })
  }
}

// What a space is, and its state on this device. Single-node: sync is off, there
// are no peers, and every identity here has full access.
fn status(store: &Store, actor: &Actor) -> Result<Value> {
  let origin = space::origin(store, Side::Reader)?;
  let managers = space::managers(store, Side::Reader)?;
  let manages = actor.identity.as_ref().is_some_and(|identity| managers.contains(identity));
  let id = store.path().file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
  Ok(json!({
    "space": id,
    "kind": origin.kind,
    "name": origin.name,
    "services": origin.services,
    "manage": origin.manage,
    "access": if manages { vec!["manage"] } else { vec!["write"] },
    "sync": "off",
    "peers": [],
  }))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{actor, brandon, call, carbon};

  #[test]
  fn refuses_replicated_mode_for_now() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("carbondb.toml");
    std::fs::write(&config, "system = \"system.db\"\n").unwrap();
    assert_eq!(Carbon::open(&config).err().unwrap().code, "NOT_BUILT");
  }

  #[test]
  fn keeps_the_secret_key_in_the_system_database_only() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    let row = carbon.system.get(Side::Reader, "SELECT device, secret FROM carbon_system_identity", &[]).unwrap();
    assert_eq!(row["device"], json!(identity.device));
    assert_eq!(row["secret"].as_str().map(str::len), Some(64));
    assert!(carbon.call_system(&call("GET [@carbon]/identities"), Value::Null).unwrap().value[0].get("secret").is_none());
  }

  #[test]
  fn refuses_calls_that_need_peers() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    for key in ["POST [@carbon]/members", "POST [@carbon]/join", "POST [@carbon]/sync/start"] {
      let error = carbon.call_space(&personal, &actor(&identity), &call(key), json!({})).unwrap_err();
      assert_eq!((error.status, error.code.as_str()), (501, "SINGLE_NODE"), "{key}");
    }
    assert_eq!(carbon.call_system(&call("POST [@carbon]/devices"), json!({})).unwrap_err().code, "SINGLE_NODE");
  }
}
