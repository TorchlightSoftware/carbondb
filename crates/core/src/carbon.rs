//! This device's CarbonDB, its root: its config, its clock, the core's timers
//! and tasks, the services it runs and the effects it can carry out, the
//! system database as its root service space, the service spaces opened so
//! far (`crate::channels`), and the watches on them (`crate::calls::watch`).
//! Every call goes through `submit`, in `crate::calls::dispatch`; a call from
//! the app comes in through `call` (`crate::app`).
//!
//! The core runs on a thread of its own (`crate::app::runtime`): the host
//! hands it the timers and tasks it runs there.
//!
//! Unless it runs single-node, each identity on this device runs a p2panda
//! node (`crate::nodes`). Effects that need peers and have no handler yet are
//! unreachable.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use serde_json::Value;

use crate::calls::dispatch::call_effect;
use crate::calls::watch::Watches;
use crate::channels::lifecycle::{Held, Starting};
use crate::channels::service::ServiceSpace;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, not_built};
use crate::nodes::Nodes;
use crate::ops::clock::Clock;
use crate::ops::effects::Effects;
use crate::ops::op::stamp;
use crate::services::carbon::{self as local, Identity};
use crate::services::{BUILTINS, Service, Services};
use crate::store::Store;
use crate::wasm::{self, Package};
use crate::{Actor, Call, Op};

// How the host runs something once a delay has passed, on the core's thread,
// handing back a way to cancel it before then.
pub type Schedule = Rc<dyn Fn(Duration, Box<dyn FnOnce()>) -> Cancel>;

// Cancels something scheduled, if it has not run yet.
pub type Cancel = Box<dyn FnOnce()>;

// Something the core runs to completion on its thread, such as a component's call.
pub type Task = Pin<Box<dyn Future<Output = ()>>>;

// How the host runs a task on the core's thread, handing back a way to drop it
// before it finishes.
pub type Spawn = Rc<dyn Fn(Task) -> Cancel>;

pub struct Carbon {
  pub(crate) config: Config,
  pub(crate) clock: Clock,
  schedule: Schedule,
  spawn: Spawn,
  services: Services,
  effects: Effects,
  pub(crate) system: Rc<ServiceSpace>,
  pub(crate) spaces: RefCell<HashMap<String, Rc<ServiceSpace>>>,
  pub(crate) watches: Watches,
  // The GETs held while their spaces start, by space.
  pub(crate) holding: RefCell<BTreeMap<String, Vec<Held>>>,
  // The spaces starting, by id.
  pub(crate) starting: RefCell<HashMap<String, Rc<Starting>>>,
  // The identities' p2panda nodes.
  pub(crate) nodes: Nodes,
}

impl Carbon {
  pub fn open(config_path: &Path, schedule: Schedule, spawn: Spawn) -> Result<Rc<Carbon>> {
    let config = Config::load(config_path)?;
    if matches!(config.sync, SyncMode::All | SyncMode::Manual) {
      return Err(not_built(&format!("The {:?} sync mode", config.sync)));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    wasm::keep_in(&config.compiled);
    let builtins = BUILTINS.iter().map(|(name, bytes)| Package::read(bytes).map(|package| (*name, package))).collect::<Result<Vec<_>>>()?;
    let services = Services::builtin()?;
    let system = Rc::new(ServiceSpace::root(Store::open(&config.system)?, local::SYSTEM_MIGRATIONS, services.get("carbon")?)?);
    let carbon = Rc::new(Carbon {
      config,
      clock: Clock::default(),
      schedule,
      spawn,
      services,
      effects: Effects::default().with("call", Rc::new(call_effect)).with("create", Rc::new(crate::channels::create_effect)).with("fetch", Rc::new(crate::ops::fetch::fetch_effect)).with("enroll", Rc::new(crate::nodes::pairing::enroll_effect)),
      system,
      spaces: RefCell::default(),
      watches: Watches::default(),
      holding: RefCell::default(),
      starting: RefCell::default(),
      nodes: Nodes::default(),
    });
    for (name, package) in builtins {
      package.compiled().wait()?;
      carbon.register(package.service(name))?;
    }
    carbon.drain(&carbon.system);
    if carbon.config.sync != SyncMode::None {
      carbon.run_nodes()?;
    }
    Ok(carbon)
  }

  pub fn services(&self) -> &Services {
    &self.services
  }

  pub fn effects(&self) -> &Effects {
    &self.effects
  }

  // The root space: the system database, where identities live.
  pub fn system(&self) -> &Rc<ServiceSpace> {
    &self.system
  }

  // Register a service ready to run, and initialize the open spaces waiting for it.
  pub fn register(self: &Rc<Self>, service: Service) -> Result<()> {
    let service = self.services.register(service)?;
    self.package_ready(&service.name);
    Ok(())
  }

  // Register an app's package under `name`, answering once its component has
  // compiled on the pool, or failed to. It is registered at once, so ops on
  // it carry its version, and its spaces initialize while it compiles.
  pub fn add(self: &Rc<Self>, name: &str, package: Package, added: Box<dyn FnOnce(Result<()>)>) {
    let compiled = package.compiled();
    if let Err(error) = self.services.register_compiling(package.service(name)) {
      return added(Err(error));
    }
    let (carbon, name) = (Rc::clone(self), name.to_string());
    let _ = self.spawn(Box::pin(async move {
      let result = compiled.ready().await;
      added(carbon.compiled(&name, result));
    }));
  }

  // A package's component has compiled, and the spaces waiting for it
  // initialize on; or it failed to, so it is not registered after all, and
  // they stop with why.
  fn compiled(self: &Rc<Self>, name: &str, result: Result<()>) -> Result<()> {
    self.services.compiled(name, &result);
    match &result {
      Ok(()) => self.package_ready(name),
      Err(error) => self.package_failed(name, error),
    }
    result
  }

  // Run a task on the core's thread, unless dropped first.
  pub(crate) fn spawn(&self, task: Task) -> Cancel {
    (self.spawn)(task)
  }

  // Run something once a delay has passed, unless cancelled first.
  pub(crate) fn after(&self, delay: Duration, then: Box<dyn FnOnce()>) -> Cancel {
    (self.schedule)(delay, then)
  }

  // Stamp a call as an op, now. Its version is that of the package the space runs.
  pub(crate) fn stamp(&self, actor: &Actor, call: &Call, input: &Value, package: &str) -> Op {
    stamp(actor, call, input, &self.services.version(package), self.clock.now())
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    local::find(&self.system.store().reader()?, slug)
  }


}
