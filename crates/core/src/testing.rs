//! Test helpers shared across the core's modules: a call from its service key,
//! a CarbonDB on a fresh directory, and an identity on it.

use serde_json::json;

use crate::call::{Actor, Call};
use crate::carbon::{Carbon, Context};
use crate::identity::Identity;

// A call from its service key: `POST [@carbon]/space` is confined, `POST [space]/ref` is not.
pub fn call(key: &str) -> Call {
  let (method, rest) = key.split_once(' ').unwrap();
  let (service, resource) = rest.trim_start_matches('[').split_once(']').unwrap();
  let confined = service.starts_with('@');
  Call { method: Some(method.into()), service: service.trim_start_matches('@').into(), resource: resource.into(), confined }
}

// A single-node CarbonDB in a temporary directory, which lives as long as the directory.
pub fn carbon() -> (tempfile::TempDir, Carbon) {
  let directory = tempfile::tempdir().unwrap();
  let config = directory.path().join("carbondb.toml");
  std::fs::write(&config, "system = \"data/system.db\"\n[sync]\nmode = \"none\"\n").unwrap();
  let carbon = Carbon::open(&config).unwrap();
  (directory, carbon)
}

pub fn alice(carbon: &Carbon) -> Identity {
  let created = carbon.call(Context::System, &Actor::default(), &call("POST [@carbon]/identity"), json!({"slug": "alice", "device": "Pixel9", "services": ["chat"]})).unwrap();
  serde_json::from_value(created.value).unwrap()
}

// An identity acting from its first device.
pub fn actor(identity: &Identity) -> Actor {
  Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) }
}
