//! This device's CarbonDB: its config, its clock, the host's timers, the
//! services it runs, the system database as its root space, the spaces opened
//! so far, and the watches on them (`watch`). Every call goes through
//! `submit`, in `dispatch`; a call from the app comes in through `call` (`app`).
//!
//! The core keeps no timers of its own: it runs on the host's thread, in
//! callback style, so the host schedules what the core asks for later, as
//! Node does with `setTimeout`.
//!
//! This is the single-node core: no p2panda and no network. Calls that need
//! peers are refused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use serde_json::Value;

use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, not_built};
use crate::op::stamp;
use crate::services::carbon::{self as local, Identity};
use crate::services::{Service, Services};
use crate::space::{self, RunState, Space};
use crate::store::Store;
use crate::watch::Watches;
use crate::{Actor, Call, Routing, Op};

// How an opened space reaches whoever asked for it.
pub type Opened = Box<dyn FnOnce(Result<Rc<Space>>)>;

// How the host runs something once a delay has passed, on the core's thread,
// handing back a way to cancel it before then.
pub type Schedule = Rc<dyn Fn(Duration, Box<dyn FnOnce()>) -> Cancel>;

// Cancels something scheduled, if it has not run yet.
pub type Cancel = Box<dyn FnOnce()>;

pub struct Carbon {
  config: Config,
  clock: Clock,
  schedule: Schedule,
  services: Services,
  system: Rc<Space>,
  spaces: RefCell<HashMap<String, Rc<Space>>>,
  pub(crate) watches: Watches,
}

impl Carbon {
  pub fn open(config_path: &Path, schedule: Schedule) -> Result<Rc<Carbon>> {
    let config = Config::load(config_path)?;
    if config.sync != SyncMode::None {
      return Err(not_built("Replicated mode"));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    let system = Rc::new(Space::root(Store::open(&config.system)?, local::SYSTEM_MIGRATIONS)?);
    let carbon = Rc::new(Carbon {
      config,
      clock: Clock::default(),
      schedule,
      services: Services::builtin(),
      system,
      spaces: RefCell::default(),
      watches: Watches::default(),
    });
    carbon.drain(&carbon.system);
    Ok(carbon)
  }

  pub fn services(&self) -> &Services {
    &self.services
  }

  // The root space: the system database, where identities live.
  pub fn system(&self) -> &Rc<Space> {
    &self.system
  }

  // Register an app's service, and set it up in every open space that names it.
  pub fn register(self: &Rc<Self>, service: Service) -> Result<()> {
    let service = self.services.register(service)?;
    let naming: Vec<Rc<Space>> = self.spaces.borrow().values().filter(|space| space.names(&service.name)).cloned().collect();
    for space in naming {
      space.set_up(&service)?;
      self.drain(&space);
    }
    Ok(())
  }

  // Run something once a delay has passed, unless cancelled first.
  pub(crate) fn after(&self, delay: Duration, then: Box<dyn FnOnce()>) -> Cancel {
    (self.schedule)(delay, then)
  }

  // Stamp a call as an op, now.
  pub(crate) fn stamp(&self, actor: &Actor, call: &Call, input: &Value) -> Op {
    stamp(actor, call, input, &self.services.version(&call.service), self.clock.now())
  }

  // Where a space's database sits: beside the system database, named by space id.
  pub(crate) fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  // A space on this device, opened on first use: `[space]` is set up first,
  // then `GET [space]/origin` names the services it runs, and each registered
  // there is set up too. A service this device does not run is refused when it
  // is called.
  pub fn with_space(self: &Rc<Self>, id: &str, opened: Opened) {
    let cached = self.spaces.borrow().get(id).cloned();
    if let Some(space) = cached {
      return opened(Ok(space));
    }
    let path = self.space_path(id);
    if !path.exists() {
      return opened(Err(space::missing(id)));
    }
    let space = match Space::open(&path, id).map(Rc::new).and_then(|space| space.set_up(&self.services.get("space")?).map(|_| space)) {
      Ok(space) => space,
      Err(error) => return opened(Err(error)),
    };
    let carbon = Rc::clone(self);
    let origin = Call::from_key("GET [space]/origin").expect("a builtin key parses");
    self.submit(&Rc::clone(&space), Actor::default(), origin, Value::Null, Box::new(move |origin| {
      let named = match origin {
        Ok(origin) => serde_json::from_value(origin.value["services"].clone()).unwrap_or_default(),
        Err(error) if error.status == 404 => vec![],
        Err(error) => return opened(Err(error)),
      };
      opened(carbon.keep_open(space, &named))
    }));
  }

  // Name a space's services, set up those this device runs, and keep it open.
  fn keep_open(self: &Rc<Self>, space: Rc<Space>, named: &[String]) -> Result<Rc<Space>> {
    space.name(named);
    for service in named.iter().filter_map(|name| self.services.find(name)) {
      space.set_up(&service)?;
    }
    self.spaces.borrow_mut().insert(space.id.clone(), Rc::clone(&space));
    self.drain(&space);
    Ok(space)
  }

  // A new space, running only `[space]` until its origin op names the rest.
  pub(crate) fn create_space(self: &Rc<Self>, id: &str) -> Result<Rc<Space>> {
    let created = Rc::new(Space::create(&self.space_path(id), id)?);
    created.set_up(&self.services.get("space")?)?;
    self.keep_open(created, &[])
  }

  // Set up the services a new space's origin names, once that origin applies:
  // every one must run on this device.
  pub(crate) fn set_up(self: &Rc<Self>, space: &Rc<Space>, names: &[String]) -> Result<()> {
    self.services.require_registered(names)?;
    self.keep_open(Rc::clone(space), names).map(|_| ())
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    local::find(&self.system.store().reader()?, slug)
  }

  // A space's RunState: idle until it is opened.
  pub fn run_state(&self, space: &str) -> RunState {
    self.opened(space).map_or(RunState::Idle, |space| space.run_state())
  }

  // A space already open on this device, the root space included.
  fn opened(&self, id: &str) -> Option<Rc<Space>> {
    if id == self.system.id {
      return Some(Rc::clone(&self.system));
    }
    self.spaces.borrow().get(id).cloned()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{actor, alice, call_now, carbon, space_now};
  use serde_json::json;

  #[test]
  fn refuses_replicated_mode_for_now() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("carbondb.toml");
    std::fs::write(&config, "system = \"system.db\"\n").unwrap();
    assert_eq!(Carbon::open(&config, crate::testing::Timers::default().schedule()).err().unwrap().code, "NOT_BUILT");
  }

  #[test]
  fn routes_by_service_name_and_refuses_services_it_does_not_run() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
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
  fn opens_a_space_naming_a_service_it_does_not_run_and_refuses_calls_to_it_until_registered() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    drop(carbon);
    let restarted = crate::testing::reopen(&directory);
    let personal = space_now(&restarted, &identity.personal).unwrap();
    let error = call_now(&restarted, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap_err();
    assert_eq!((error.status, error.details), (404, Some(json!({"service": "notes"}))));
    crate::testing::register_notes(&restarted);
    assert_eq!(call_now(&restarted, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap().value, json!([]));
  }
}
