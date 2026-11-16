//! Channels: each service a channel names runs on this device as a service
//! space of its own, one SQLite file and op log (`service`), which starts
//! before it applies anything (`lifecycle`). A channel's own space is the
//! service space of its `[channel]` service, and the system root that of
//! `[@carbon]`. Calls reach a channel's services through the names it gives
//! them and its refs.

pub mod lifecycle;
pub mod service;

use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};

use crate::calls::service_call::ServiceCall;
use crate::carbon::Carbon;
use crate::error::{Result, bad_request, not_found};
use crate::ops::clock;
use crate::ops::effects::Made;
use crate::ops::oplog::{self, LogRow};
use crate::{Call, Db, Reply};
use service::{RunState, Runs, ServiceSpace};

// How an opened space reaches whoever asked for it.
pub type Opened = Box<dyn FnOnce(Result<Rc<ServiceSpace>>)>;

impl Carbon {
  // Where a space's database sits: beside the system database, named by space id.
  pub(crate) fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  // A space on this device, opened on first use with the service this device
  // recorded it runs, and started (`lifecycle`).
  pub fn with_space(self: &Rc<Self>, id: &str, opened: Opened) {
    opened(self.space(id))
  }

  pub(crate) fn space(self: &Rc<Self>, id: &str) -> Result<Rc<ServiceSpace>> {
    if let Some(space) = self.opened(id) {
      return Ok(space);
    }
    let runs = self.held(id)?.ok_or_else(|| service::missing(id))?;
    let space = Rc::new(ServiceSpace::open(&self.space_path(id), id, runs)?);
    self.spaces.borrow_mut().insert(space.id.clone(), Rc::clone(&space));
    self.initialize(&space);
    Ok(space)
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

  // A new space on this device, running a registered package, recorded here
  // with what it runs, kept open and initializing. One whose package is here
  // is set up at once, and deleted again when that fails; one whose package
  // is still compiling is set up once it has.
  pub(crate) fn create_space(self: &Rc<Self>, id: &str, runs: Runs) -> Result<Rc<ServiceSpace>> {
    self.services().get(&runs.package)?;
    let created = Rc::new(ServiceSpace::create(&self.space_path(id), id, runs.clone())?);
    self.system.store().run("INSERT INTO carbon_system_space (id, package, name, channel) VALUES (?, ?, ?, ?)", &[json!(id), json!(runs.package), json!(runs.name), json!(runs.channel)])?;
    self.spaces.borrow_mut().insert(id.to_string(), Rc::clone(&created));
    self.begin_starting(&created);
    if let Err(error) = self.prepare(&created) {
      self.starting_now(id, None);
      drop(created);
      self.forget(id)?;
      return Err(error);
    }
    Ok(created)
  }

  // The channel a space belongs to, through which its calls to other services resolve.
  pub(crate) fn channel_of(&self, space: &ServiceSpace) -> Result<String> {
    space.runs.channel.clone().ok_or_else(|| not_found(format!("{} belongs to no channel, so it calls no other service", space.called())))
  }

  // The space a call reaches from a space or a channel: the space itself when
  // it runs the call's service, or the service its channel names for it.
  pub(crate) fn service_space(self: &Rc<Self>, id: &str, call: &Call) -> Result<Rc<ServiceSpace>> {
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
    let accepted = self.space(id).and_then(|space| crate::calls::dispatch::accepting(&space).map(|_| space));
    received(accepted.map(|space| {
      self.clock.observe(row.op.time);
      self.log(&space, row)
    }))
  }

  // A space's RunState: idle until it is opened.
  pub fn run_state(&self, space: &str) -> RunState {
    self.opened(space).map_or(RunState::Idle, |space| space.run_state())
  }

  // A space already open on this device, the root space included.
  fn opened(&self, id: &str) -> Option<Rc<ServiceSpace>> {
    if id == self.system.id {
      return Some(Rc::clone(&self.system));
    }
    self.spaces.borrow().get(id).cloned()
  }
  // The spaces open on this device, the root space aside.
  pub(crate) fn open_spaces(&self) -> Vec<Rc<ServiceSpace>> {
    self.spaces.borrow().values().cloned().collect()
  }

}

// The spaces `create` effects made.
fn created(made: &[Made]) -> Vec<String> {
  made.iter().filter(|made| made.name == "create").filter_map(|made| made.output.as_ref().ok()?["space"].as_str().map(str::to_string)).collect()
}

// The `create` effect: the space of a new channel or service, created on this
// device running the package named. A channel's space belongs to itself; a
// service's to the channel of the space that creates it.
pub(crate) fn create_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
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
