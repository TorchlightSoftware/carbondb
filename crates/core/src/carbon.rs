//! This device's CarbonDB: its config, its clock, the host's timers, the
//! services it runs, the effects it can carry out, the system database as its
//! root space, the spaces opened so far, and the watches on them (`watch`).
//! Every call goes through `submit`, in `dispatch`; a call from the app comes
//! in through `call` (`app`).
//!
//! Each space runs one service. Which one, under what name and in which
//! channel, is this device's own record, kept in the system database as each
//! space is created here; a call to `[name]` reaches the space its channel
//! names for it.
//!
//! The core keeps no timers of its own: it runs on the host's thread, in
//! callback style, so the host schedules what the core asks for later, as
//! Node does with `setTimeout`.
//!
//! This is the single-node core: no p2panda and no network. Effects that need
//! peers have no handler here, and are unreachable.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::dispatch::call_effect;
use crate::error::{Result, bad_request, not_built, not_found};
use crate::ops::clock::{self, Clock};
use crate::ops::effects::{Effects, Made};
use crate::ops::op::stamp;
use crate::ops::oplog::{self, LogRow};
use crate::config::{Config, SyncMode};
use crate::service_call::ServiceCall;
use crate::services::carbon::{self as local, Identity};
use crate::services::{Service, Services};
use crate::space::{self, RunState, Runs, Space};
use crate::store::Store;
use crate::watch::Watches;
use crate::{Actor, Call, Db, Op, Reply};

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
  effects: Effects,
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
    let services = Services::builtin();
    let system = Rc::new(Space::root(Store::open(&config.system)?, local::SYSTEM_MIGRATIONS, services.get("carbon")?)?);
    let carbon = Rc::new(Carbon {
      config,
      clock: Clock::default(),
      schedule,
      services,
      effects: Effects::default().with("call", Rc::new(call_effect)).with("create", Rc::new(create_effect)),
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

  pub fn effects(&self) -> &Effects {
    &self.effects
  }

  // The root space: the system database, where identities live.
  pub fn system(&self) -> &Rc<Space> {
    &self.system
  }

  // Register an app's service, and set it up in every open space that runs it.
  pub fn register(self: &Rc<Self>, service: Service) -> Result<()> {
    let service = self.services.register(service)?;
    let running: Vec<Rc<Space>> = self.spaces.borrow().values().filter(|space| space.not_set_up().as_deref() == Some(service.name.as_str())).cloned().collect();
    for space in running {
      space.set_up(&service)?;
      self.drain(&space);
    }
    Ok(())
  }

  // Run something once a delay has passed, unless cancelled first.
  pub(crate) fn after(&self, delay: Duration, then: Box<dyn FnOnce()>) -> Cancel {
    (self.schedule)(delay, then)
  }

  // Stamp a call as an op, now. Its version is that of the package the space runs.
  pub(crate) fn stamp(&self, actor: &Actor, call: &Call, input: &Value, package: &str) -> Op {
    stamp(actor, call, input, &self.services.version(package), self.clock.now())
  }

  // Where a space's database sits: beside the system database, named by space id.
  pub(crate) fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  // A space on this device, opened on first use with the service this device
  // recorded it runs, set up here when it is registered. A space whose service
  // is not registered opens, and its ops wait until it is.
  pub fn with_space(self: &Rc<Self>, id: &str, opened: Opened) {
    opened(self.space(id))
  }

  pub(crate) fn space(self: &Rc<Self>, id: &str) -> Result<Rc<Space>> {
    if let Some(space) = self.opened(id) {
      return Ok(space);
    }
    let runs = self.held(id)?.ok_or_else(|| space::missing(id))?;
    let space = Rc::new(Space::open(&self.space_path(id), id, runs)?);
    self.set_up(&space)?;
    Ok(self.keep_open(space))
  }

  // What this device recorded a space runs, if it holds the space.
  fn held(&self, id: &str) -> Result<Option<Runs>> {
    let row = self.system.store().reader()?.get("SELECT package, name, channel FROM carbon_system_space WHERE id = ?", &[json!(id)])?;
    Ok((!row.is_null()).then(|| Runs {
      package: row["package"].as_str().unwrap_or_default().to_string(),
      name: row["name"].as_str().unwrap_or_default().to_string(),
      channel: row["channel"].as_str().map(str::to_string),
    }))
  }

  // Set a space's service up, when it is registered here.
  fn set_up(&self, space: &Rc<Space>) -> Result<()> {
    self.services.find(&space.runs.package).map_or(Ok(()), |service| space.set_up(&service))
  }

  // Keep a space open, and apply what is pending in it.
  fn keep_open(self: &Rc<Self>, space: Rc<Space>) -> Rc<Space> {
    self.spaces.borrow_mut().insert(space.id.clone(), Rc::clone(&space));
    self.drain(&space);
    space
  }

  // A new space on this device, running a registered package, recorded here
  // with what it runs. One whose service fails to set up is deleted again.
  pub(crate) fn create_space(self: &Rc<Self>, id: &str, runs: Runs) -> Result<Rc<Space>> {
    let service = self.services.get(&runs.package)?;
    let created = Rc::new(Space::create(&self.space_path(id), id, runs.clone())?);
    self.system.store().run("INSERT INTO carbon_system_space (id, package, name, channel) VALUES (?, ?, ?, ?)", &[json!(id), json!(runs.package), json!(runs.name), json!(runs.channel)])?;
    if let Err(error) = created.set_up(&service) {
      drop(created);
      self.forget(id)?;
      return Err(error);
    }
    Ok(self.keep_open(created))
  }

  // The channel a space belongs to, through which its calls to other services resolve.
  pub(crate) fn channel_of(&self, space: &Space) -> Result<String> {
    space.runs.channel.clone().ok_or_else(|| not_found(format!("Space {} belongs to no channel, so it calls no other service", space.id)))
  }

  // The space a call reaches from a space or a channel: the space itself when
  // it runs the call's service, or the service its channel names for it.
  pub(crate) fn service_space(self: &Rc<Self>, id: &str, call: &Call) -> Result<Rc<Space>> {
    let space = self.space(id)?;
    if space.answers(call) || space.is_root() {
      return Ok(space);
    }
    let channel = self.channel_of(&space)?;
    let named = self.named(&channel, &call.service)?;
    self.space(&named)
  }

  // The space a channel names for `[name]`: its own space for `[channel]`.
  pub fn named(self: &Rc<Self>, channel: &str, name: &str) -> Result<String> {
    if name == "channel" {
      return Ok(channel.to_string());
    }
    let found = self.read_tables(channel, |db| db.get("SELECT space FROM service WHERE name = ?", &[json!(name)]))?;
    found["space"].as_str().map(str::to_string).ok_or_else(|| not_found(format!("[{name}] is not a service this channel names")).with_details(json!({"service": name})))
  }

  // Delete this device's copy of a space: closed, its file removed, and
  // forgotten here.
  pub(crate) fn forget(&self, id: &str) -> Result<()> {
    self.spaces.borrow_mut().remove(id);
    self.system.store().run("DELETE FROM carbon_system_space WHERE id = ?", &[json!(id)])?;
    for suffix in ["", "-wal", "-shm"] {
      let path = PathBuf::from(format!("{}{suffix}", self.space_path(id).display()));
      if path.exists() {
        std::fs::remove_file(&path).map_err(|error| bad_request(format!("Cannot delete {}: {error}", path.display())))?;
      }
    }
    Ok(())
  }

  // Delete the spaces a run created, by the `create` effects it made, and
  // every space their own ops created in turn.
  pub(crate) fn unmake(self: &Rc<Self>, made: &[Made]) -> Result<()> {
    created(made).iter().try_for_each(|id| self.unmake_space(id))
  }

  fn unmake_space(self: &Rc<Self>, id: &str) -> Result<()> {
    let rows = oplog::rows(self.space(id)?.store())?;
    let made: Vec<Made> = rows.into_iter().flat_map(|row| row.effects.unwrap_or_default()).collect();
    self.unmake(&made)?;
    self.forget(id)
  }

  // A row another device sent for a space this device holds: once its time is
  // admitted and its space accepts ops, the time moves this device's clock
  // past it, and it is logged like any other op and applied in the shared order.
  pub fn receive(self: &Rc<Self>, id: &str, row: LogRow, received: Box<dyn FnOnce(Result<()>)>) {
    if let Err(refused) = clock::admit(row.op.time, clock::wall()) {
      return received(Err(refused));
    }
    let accepted = self.space(id).and_then(|space| crate::dispatch::accepting(&space).map(|_| space));
    received(accepted.map(|space| {
      self.clock.observe(row.op.time);
      self.log(&space, row)
    }))
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

// The `create` effect: the space of a new channel or service, created on this
// device running the package named. A channel's space belongs to itself; a
// service's to the channel of the space that creates it.
// The spaces `create` effects made.
fn created(made: &[Made]) -> Vec<String> {
  made.iter().filter(|made| made.name == "create").filter_map(|made| made.output.as_ref().ok()?["space"].as_str().map(str::to_string)).collect()
}

fn create_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
  reply(create(&caller, &input))
}

fn create(caller: &ServiceCall, input: &Value) -> Result<Value> {
  let carbon = caller.carbon();
  let (id, package) = (input["space"].as_str().unwrap_or_default(), input["package"].as_str().unwrap_or_default().to_string());
  let name = input["name"].as_str().map_or_else(|| package.clone(), str::to_string);
  let channel = match package.as_str() {
    "channel" => Some(id.to_string()),
    _ => caller.space().runs.channel.clone(),
  };
  carbon.create_space(id, Runs { package: package.clone(), name, channel })?;
  Ok(json!({"space": id, "version": carbon.services().version(&package)}))
}
