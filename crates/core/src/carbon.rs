//! This device's CarbonDB: its config, its clock, the services it runs, the
//! system database as its root space, and the spaces opened so far. Every call
//! goes through `submit`, in `dispatch`.
//!
//! This is the single-node core: no p2panda and no network. Calls that need
//! peers are refused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;

use crate::builtins;
use crate::call::{Actor, Call};
use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, not_built};
use crate::identity::{self, Identity};
use crate::op::Op;
use crate::services::Services;
use crate::space::{self, Space};

pub struct Carbon {
  config: Config,
  clock: Clock,
  services: Services,
  system: Rc<Space>,
  spaces: RefCell<HashMap<String, Rc<Space>>>,
}

impl Carbon {
  pub fn open(config_path: &Path) -> Result<Rc<Carbon>> {
    let config = Config::load(config_path)?;
    if config.sync != SyncMode::None {
      return Err(not_built("Replicated mode"));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    let system = Rc::new(Space::root(identity::open_system(&config.system)?)?);
    Ok(Rc::new(Carbon { config, clock: Clock::default(), services: builtins::services(), system, spaces: RefCell::default() }))
  }

  pub fn services(&self) -> &Services {
    &self.services
  }

  // The root space: the system database, where identities live.
  pub fn system(&self) -> &Rc<Space> {
    &self.system
  }

  // Stamp a call as an op, now.
  pub(crate) fn stamp(&self, actor: &Actor, call: &Call, input: &Value) -> Op {
    Op::stamp(actor, call, input, self.services.version(&call.service), self.clock.now())
  }

  // Where a space's database sits: beside the system database, named by space id.
  fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  // A space on this device, opened on first use with every service its origin
  // names provisioned. A space naming a service this device does not run is refused.
  pub fn space(&self, id: &str) -> Result<Rc<Space>> {
    if let Some(space) = self.spaces.borrow().get(id) {
      return Ok(Rc::clone(space));
    }
    let path = self.space_path(id);
    if !path.exists() {
      return Err(space::missing(id));
    }
    let opened = Rc::new(Space::open(&path, id)?);
    self.provision(&opened, &["space".to_string()])?;
    let origin = builtins::space::origin(&opened.store().reader()?)?;
    self.provision(&opened, &origin.services)?;
    self.spaces.borrow_mut().insert(id.to_string(), Rc::clone(&opened));
    Ok(opened)
  }

  // A new space, running only `[space]` until its origin op names the rest.
  pub(crate) fn create_space(&self, id: &str) -> Result<Rc<Space>> {
    let created = Rc::new(Space::create(&self.space_path(id), id)?);
    self.provision(&created, &["space".to_string()])?;
    self.spaces.borrow_mut().insert(id.to_string(), Rc::clone(&created));
    Ok(created)
  }

  // Provision the services a space names: every one must run on this device.
  pub(crate) fn provision(&self, space: &Space, names: &[String]) -> Result<()> {
    for name in names {
      space.provision(self.services.get(name)?.as_ref())?;
    }
    Ok(())
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    identity::find(&self.system.store().reader()?, slug)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{actor, alice, call_now, carbon};
  use serde_json::json;

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
    let personal = carbon.space(&identity.personal).unwrap();
    let system = Rc::clone(carbon.system());
    assert_eq!(call_now(&carbon, &personal, &actor(&identity), "GET [device]/refs", Value::Null).unwrap().value[0]["name"], json!("Pixel9"));
    assert_eq!(call_now(&carbon, &system, &Actor::default(), "GET [@carbon]/identities", Value::Null).unwrap().value[0]["slug"], json!("alice"));
    let refused = [
      (&personal, "GET [@elsewhere]/x"),
      (&personal, "GET [identity]/origin"),
      (&personal, "GET [chat]/messages"),
      (&personal, "GET [@device]/refs"),
      (&system, "GET [space]/refs"),
      (&system, "GET [@carbon]/status"),
    ];
    for (space, key) in refused {
      assert_eq!(call_now(&carbon, space, &actor(&identity), key, Value::Null).unwrap_err().status, 404, "{key}");
    }
  }

  #[test]
  fn refuses_a_space_naming_a_service_it_does_not_run() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    drop(carbon);
    let config = directory.path().join("carbondb.toml");
    let restarted = Carbon::open(&config).unwrap();
    let error = restarted.space(&identity.personal).err().unwrap();
    assert_eq!((error.status, error.details), (404, Some(json!({"service": "notes"}))));
  }
}
