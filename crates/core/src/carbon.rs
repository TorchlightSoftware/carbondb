//! This device's CarbonDB: its config, its clock, the services it runs, the
//! system database as its root space, the spaces opened so far, and who
//! listens for their commits and RunStates. Every call goes through `submit`,
//! in `dispatch`.
//!
//! This is the single-node core: no p2panda and no network. Calls that need
//! peers are refused.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;

use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, internal, not_built};
use crate::op::stamp;
use crate::services::carbon::{self as local, Identity};
use crate::services::{Service, Services};
use crate::space::{self, RunState, Space};
use crate::store::Store;
use crate::{Actor, Call, Op};

// Called with the tables each commit to a space wrote.
pub type Listener = Rc<dyn Fn(&[String])>;

// Called with a space's RunState each time it changes.
pub type RunStateListener = Rc<dyn Fn(&RunState)>;

// How an opened space reaches whoever asked for it.
pub type Opened = Box<dyn FnOnce(Result<Rc<Space>>)>;

pub struct Carbon {
  config: Config,
  clock: Clock,
  services: Services,
  system: Rc<Space>,
  spaces: RefCell<HashMap<String, Rc<Space>>>,
  listeners: RefCell<HashMap<String, Vec<Listener>>>,
  run_state_listeners: RefCell<HashMap<String, Vec<RunStateListener>>>,
}

impl Carbon {
  pub fn open(config_path: &Path) -> Result<Rc<Carbon>> {
    let config = Config::load(config_path)?;
    if config.sync != SyncMode::None {
      return Err(not_built("Replicated mode"));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    let system = Rc::new(Space::root(Store::open(&config.system)?, local::SYSTEM_MIGRATIONS)?);
    let carbon = Rc::new(Carbon {
      config,
      clock: Clock::default(),
      services: Services::builtin(),
      system,
      spaces: RefCell::default(),
      listeners: RefCell::default(),
      run_state_listeners: RefCell::default(),
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

  // Stamp a call as an op, now.
  pub(crate) fn stamp(&self, actor: &Actor, call: &Call, input: &Value) -> Op {
    stamp(actor, call, input, &self.services.version(&call.service), self.clock.now())
  }

  // Where a space's database sits: beside the system database, named by space id.
  fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  // The ids of every space on this device.
  pub fn space_ids(&self) -> Result<Vec<String>> {
    let entries = std::fs::read_dir(self.config.directory()).map_err(|error| internal(format!("Cannot list spaces: {error}")))?;
    let mut ids: Vec<String> = entries
      .filter_map(|entry| entry.ok().map(|entry| entry.path()))
      .filter(|path| path.extension().is_some_and(|extension| extension == "db") && *path != self.config.system)
      .filter_map(|path| path.file_stem().map(|stem| stem.to_string_lossy().to_string()))
      .collect();
    ids.sort();
    Ok(ids)
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

  // Hear every commit to a space, with the tables it wrote, whether or not it is open yet.
  pub fn listen(&self, space: &str, listener: Listener) {
    self.listeners.borrow_mut().entry(space.to_string()).or_default().push(listener);
  }

  // Tell every listener what a commit to a space wrote.
  pub(crate) fn committed(&self, space: &str, tables: &[String]) {
    let listeners = self.listeners.borrow().get(space).cloned().unwrap_or_default();
    for listener in listeners {
      listener(tables);
    }
  }

  // A space's RunState: idle until it is opened.
  pub fn run_state(&self, space: &str) -> RunState {
    self.opened(space).map_or(RunState::Idle, |space| space.run_state())
  }

  // Hear every change of a space's RunState, whether or not it is open yet.
  pub fn listen_run_state(&self, space: &str, listener: RunStateListener) {
    self.run_state_listeners.borrow_mut().entry(space.to_string()).or_default().push(listener);
  }

  // Move a space to a RunState, and tell whoever listens.
  pub(crate) fn set_run_state(&self, space: &Space, state: RunState) {
    space.set_run_state(state.clone());
    let listeners = self.run_state_listeners.borrow().get(&space.id).cloned().unwrap_or_default();
    for listener in listeners {
      listener(&state);
    }
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
    assert_eq!(Carbon::open(&config).err().unwrap().code, "NOT_BUILT");
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
    let restarted = Carbon::open(&directory.path().join("carbondb.toml")).unwrap();
    assert_eq!(restarted.space_ids().unwrap(), {
      let mut ids = vec![identity.space.clone(), identity.personal.clone()];
      ids.sort();
      ids
    });
    let personal = space_now(&restarted, &identity.personal).unwrap();
    let error = call_now(&restarted, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap_err();
    assert_eq!((error.status, error.details), (404, Some(json!({"service": "notes"}))));
    crate::testing::register_notes(&restarted);
    assert_eq!(call_now(&restarted, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap().value, json!([]));
  }
}
