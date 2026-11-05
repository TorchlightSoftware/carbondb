//! This device's CarbonDB: its config, the system database and its clock. It
//! opens spaces, stamps ops, and routes every call it answers by service name:
//! `[@carbon]` to the device's local service, and the replicated builtins to
//! theirs. App services are routed by the runtime that hosts them.
//!
//! This is the single-node core: no p2panda, no network and no op log. Calls
//! that need peers are refused.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::builtins;
use crate::call::{Actor, Call, Outcome};
use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, not_built, not_found};
use crate::hash::hash;
use crate::identity::{self, Identity};
use crate::space;
use crate::store::Store;

// When an op happened, and its hash over the envelope and that time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stamp {
  pub time: u64,
  pub hash: String,
}

// Where a call is made: on the system handle, or in a space.
#[derive(Clone, Copy)]
pub enum Context<'a> {
  System,
  Space(&'a Store),
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
    let system = identity::open_system(&config.system)?;
    Ok(Carbon { config, system, clock: Clock::default() })
  }

  // Stamp an op's envelope with a time, and hash the two together.
  pub fn stamp(&self, envelope: &Value) -> Stamp {
    let time = self.clock.now();
    Stamp { time, hash: hash(&json!({"envelope": envelope, "time": time})) }
  }

  pub(crate) fn system(&self) -> &Store {
    &self.system
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
    space::open(&path)
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    identity::find(&self.system, slug)
  }

  // A call this core answers, routed by its service name.
  pub fn call(&self, context: Context, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
    match (call.confined, context) {
      (true, _) if call.service == "carbon" => builtins::carbon::handle(self, context, actor, call, input),
      (true, _) => Err(not_found(format!("No local service [@{}] on this device", call.service))),
      (false, Context::Space(store)) => builtins::handle(store, actor, call, input),
      (false, Context::System) => Err(not_found(format!("The system handle answers only [@carbon], not {}", call.key()))),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{actor, alice, call, carbon};

  #[test]
  fn refuses_replicated_mode_for_now() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("carbondb.toml");
    std::fs::write(&config, "system = \"system.db\"\n").unwrap();
    assert_eq!(Carbon::open(&config).err().unwrap().code, "NOT_BUILT");
  }

  #[test]
  fn routes_by_service_name_and_refuses_services_it_does_not_run() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    assert_eq!(carbon.call(Context::Space(&personal), &actor(&identity), &call("GET [device]/refs"), Value::Null).unwrap().value[0]["name"], json!("Pixel9"));
    assert_eq!(carbon.call(Context::System, &Actor::default(), &call("GET [@carbon]/identities"), Value::Null).unwrap().value[0]["slug"], json!("alice"));
    let refused = [
      (Context::Space(&personal), "GET [@elsewhere]/x"),
      (Context::System, "GET [space]/refs"),
      (Context::System, "GET [@carbon]/status"),
    ];
    for (context, key) in refused {
      assert_eq!(carbon.call(context, &actor(&identity), &call(key), Value::Null).unwrap_err().status, 404, "{key}");
    }
  }
}
