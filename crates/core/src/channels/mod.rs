//! Channels: a channel on this device is one log every service in it shares
//! (`channel`), and a store for each of those services, holding its tables
//! and what it did with each op addressed to it (`service`), which starts
//! before it applies anything (`lifecycle`). Every channel runs its own
//! `[channel]`, whose store is named by the channel's id, and `[@carbon]`; the
//! system is a local channel whose `[@carbon]` store is the system database.
//! Calls reach a channel's services through the names its `[channel]` gives
//! them.

pub mod channel;
pub mod lifecycle;
pub mod service;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::json;

use crate::Call;
use crate::calls::call::address;
use crate::carbon::Carbon;
use crate::error::{Result, ServiceError, bad_request, conflict, not_found};
use crate::ops::clock;
use crate::ops::effects::Made;
use crate::ops::oplog::{self, LogRow};
use crate::ops::pointers::Pointer;
use channel::{Channel, Kind, held_in};
use service::{CARBON, RunState, Runs, ServiceStore};

// How an opened store reaches whoever asked for it.
pub type Opened = Box<dyn FnOnce(Result<Rc<ServiceStore>>)>;

// The channels and stores open on this device, by id.
#[derive(Default)]
pub(crate) struct Open {
  channels: RefCell<HashMap<String, Rc<Channel>>>,
  stores: RefCell<HashMap<String, Rc<ServiceStore>>>,
}

impl Carbon {
  // Where an identity's files sit: a directory of its own in the data
  // directory. The system's sit in the data directory itself.
  pub(crate) fn directory_of(&self, identity: Option<&str>) -> PathBuf {
    match identity {
      Some(identity) => self.config.directory().join(identity),
      None => self.config.directory().to_path_buf(),
    }
  }

  // A channel this device holds, opened on first use: one whose log is in an
  // identity's directory.
  pub(crate) fn channel(&self, id: &str) -> Result<Rc<Channel>> {
    if let Some(channel) = self.open.channels.borrow().get(id) {
      return Ok(Rc::clone(channel));
    }
    let identity = self.holder(id).ok_or_else(|| not_found(format!("No channel {id} on this device")))?;
    let channel = Rc::new(Channel::open(&self.directory_of(Some(&identity)), id, Some(&identity), Kind::Replicated)?);
    self.open.channels.borrow_mut().insert(id.to_string(), Rc::clone(&channel));
    Ok(channel)
  }

  // Keep a channel open, as the system's is from the start.
  pub(crate) fn keep_channel(&self, channel: &Rc<Channel>) {
    self.open.channels.borrow_mut().insert(channel.id.clone(), Rc::clone(channel));
  }

  // The identity on this device whose directory holds a channel's log.
  fn holder(&self, id: &str) -> Option<String> {
    let entries = std::fs::read_dir(self.config.directory()).ok()?;
    let identities = entries.filter_map(|entry| entry.ok()).filter(|entry| entry.path().is_dir()).filter_map(|entry| entry.file_name().to_str().map(str::to_string));
    identities.into_iter().find(|identity| held_in(&self.directory_of(Some(identity))).iter().any(|held| held == id))
  }

  // A new channel on this device, for an identity: its log and its `[channel]`'s store.
  pub(crate) fn create_channel(self: &Rc<Self>, id: &str, identity: &str) -> Result<Rc<Channel>> {
    let directory = self.directory_of(Some(identity));
    if channel::log_path(&directory, id).exists() {
      return Err(conflict(format!("A channel {id} is already on this device")));
    }
    let created = Rc::new(Channel::open(&directory, id, Some(identity), Kind::Replicated)?);
    self.keep_channel(&created);
    self.channel_store(&created)?;
    Ok(created)
  }

  // A store on this device: one open, or a held channel's `[channel]` store.
  pub(crate) fn store(self: &Rc<Self>, id: &str) -> Result<Rc<ServiceStore>> {
    if let Some(store) = self.opened(id) {
      return Ok(store);
    }
    let channel = self.channel(id).map_err(|_| service::missing(id))?;
    self.channel_store(&channel)
  }

  // A store on this device, opened on first use and started (`lifecycle`).
  pub fn with_store(self: &Rc<Self>, id: &str, opened: Opened) {
    opened(self.store(id))
  }

  // A channel's own `[channel]`, whose store is named by the channel's id.
  pub(crate) fn channel_store(self: &Rc<Self>, channel: &Rc<Channel>) -> Result<Rc<ServiceStore>> {
    let id = channel.id.clone();
    self.open_store(channel, &id, Runs { package: "channel".into(), name: "channel".into() })
  }

  // A channel's `[@carbon]`, the system database's for the system.
  pub(crate) fn carbon_store(self: &Rc<Self>, channel: &Rc<Channel>) -> Result<Rc<ServiceStore>> {
    if let Some(store) = self.opened(&channel.carbon()) {
      return Ok(store);
    }
    let store = Rc::new(ServiceStore::carbon(channel, self.services().get("carbon")?)?);
    self.keep(&store)?;
    self.drain(&store);
    Ok(store)
  }

  // The store of a service a channel names, by the name it gives it.
  pub(crate) fn named_store(self: &Rc<Self>, channel: &Rc<Channel>, name: &str) -> Result<Rc<ServiceStore>> {
    match name {
      "channel" => self.channel_store(channel),
      CARBON => self.carbon_store(channel),
      _ => {
        let (id, package) = self.service_named(&channel.id, name)?;
        self.open_store(channel, &id, Runs { package, name: name.to_string() })
      }
    }
  }

  // A store opened on first use, pointing to what the channel's log
  // addresses to it, and started (`lifecycle`).
  fn open_store(self: &Rc<Self>, channel: &Rc<Channel>, id: &str, runs: Runs) -> Result<Rc<ServiceStore>> {
    if let Some(store) = self.opened(id) {
      return Ok(store);
    }
    let store = Rc::new(ServiceStore::open(channel, id, runs)?);
    self.keep(&store)?;
    self.initialize(&store);
    Ok(store)
  }

  // A new store for a service of a channel, running a registered package,
  // kept open and initializing. One whose package is here is set up at once,
  // and deleted again when that fails; one whose package is still compiling
  // is set up once it has.
  pub(crate) fn create_service(self: &Rc<Self>, channel: &Rc<Channel>, id: &str, runs: Runs) -> Result<Rc<ServiceStore>> {
    self.services().get(&runs.package)?;
    let created = Rc::new(ServiceStore::create(channel, id, runs)?);
    self.keep(&created)?;
    self.begin_starting(&created);
    if let Err(error) = self.prepare(&created) {
      self.starting_now(id, None);
      drop(created);
      self.forget_store(id)?;
      return Err(error);
    }
    Ok(created)
  }

  // Keep a store open, pointing to every op its channel's log addresses to it.
  fn keep(&self, store: &Rc<ServiceStore>) -> Result<()> {
    self.open.stores.borrow_mut().insert(store.id.clone(), Rc::clone(store));
    crate::calls::dispatch::take_up(store)
  }

  // The store a call reaches from a store or a channel: the store itself when
  // it runs the call's service, or the one its channel names for it.
  pub(crate) fn service_space(self: &Rc<Self>, id: &str, call: &Call) -> Result<Rc<ServiceStore>> {
    let store = self.store(id)?;
    if store.answers(call) {
      return Ok(store);
    }
    let channel = Rc::clone(&store.channel);
    self.named_store(&channel, &address(call))
  }

  // The store id a channel names for `[name]`: its own for `[channel]`.
  pub fn named(self: &Rc<Self>, channel: &str, name: &str) -> Result<String> {
    match name {
      "channel" => Ok(channel.to_string()),
      _ => self.service_named(channel, name).map(|(id, _)| id),
    }
  }

  // The store id and package of a service a channel names, read from its `[channel]`.
  fn service_named(self: &Rc<Self>, channel: &str, name: &str) -> Result<(String, String)> {
    let found = self.read_tables(channel, |db| db.get("SELECT space, package FROM service WHERE name = ?", &[json!(name)]))?;
    match (found["space"].as_str(), found["package"].as_str()) {
      (Some(id), Some(package)) => Ok((id.to_string(), package.to_string())),
      _ => Err(not_found(format!("[{name}] is not a service this channel names")).with_details(json!({"service": name}))),
    }
  }

  // Delete this device's copy of a service: closed, and its file removed.
  pub(crate) fn forget_store(&self, id: &str) -> Result<()> {
    let removed = self.open.stores.borrow_mut().remove(id);
    removed.map_or(Ok(()), |store| remove_files(store.channel.store_path(id)))
  }

  // Delete this device's copy of a channel: its log and its own stores
  // removed, and its p2panda space no longer followed.
  pub(crate) fn forget_channel(&self, id: &str) -> Result<()> {
    self.unfollow(id);
    let removed = self.open.channels.borrow_mut().remove(id);
    let Some(channel) = removed else { return Ok(()) };
    [id.to_string(), channel.carbon()].iter().try_for_each(|store| {
      self.open.stores.borrow_mut().remove(store);
      remove_files(channel.store_path(store))
    })?;
    remove_files(channel::log_path(channel.directory(), id))
  }

  // Delete everything of an identity on this device: its open channels, and its directory.
  pub(crate) fn forget_identity(&self, identity: &str) -> Result<()> {
    let held: Vec<String> = self.open.channels.borrow().values().filter(|channel| channel.identity.as_deref() == Some(identity)).map(|channel| channel.id.clone()).collect();
    held.iter().try_for_each(|channel| self.forget_channel(channel))?;
    self.open.stores.borrow_mut().retain(|_, store| store.channel.identity.as_deref() != Some(identity));
    let directory = self.directory_of(Some(identity));
    match directory.exists() {
      true => std::fs::remove_dir_all(&directory).map_err(|error| bad_request(format!("Cannot delete {}: {error}", directory.display()))),
      false => Ok(()),
    }
  }

  // Delete what a refused op's first run created, by the `create` effects it made.
  pub(crate) fn unmake(&self, made: &[Made]) -> Result<()> {
    let created = made.iter().filter(|made| made.name == "create").filter_map(|made| made.output.as_ref().ok());
    created.into_iter().try_for_each(|created| match (created["space"].as_str(), created["channel"].as_bool()) {
      (Some(id), Some(true)) => self.forget_channel(id),
      (Some(id), _) => self.forget_store(id),
      _ => Ok(()),
    })
  }

  // A row another device sent for a channel this device holds: once its time
  // is admitted, the time moves this device's clock past it, and it is
  // logged like any other op and applied in the shared order. One for a
  // service this channel does not name here yet waits in the log until it
  // does, and its store, once opened, points to it.
  pub fn receive(self: &Rc<Self>, id: &str, row: LogRow, received: Box<dyn FnOnce(Result<()>)>) {
    if let Err(refused) = clock::admit(row.op.time, clock::wall()) {
      return received(Err(refused));
    }
    received(self.channel(id).and_then(|channel| self.log_received(&channel, row)))
  }

  fn log_received(self: &Rc<Self>, channel: &Rc<Channel>, row: LogRow) -> Result<()> {
    match row.service().and_then(|name| self.named_store(channel, &name)) {
      Ok(store) => self.receive_in(&store, row),
      Err(_) => {
        self.clock.observe(row.op.time);
        oplog::append(channel.log(), &row).map(|_| ())
      }
    }
  }

  // A received row for a store this device holds: logged and applied in its turn, unless the store is stopped.
  pub(crate) fn receive_in(self: &Rc<Self>, store: &Rc<ServiceStore>, row: LogRow) -> Result<()> {
    crate::calls::dispatch::accepting(store)?;
    self.clock.observe(row.op.time);
    self.log(store, row);
    Ok(())
  }

  // A store's RunState: idle until it is opened.
  pub fn run_state(&self, store: &str) -> RunState {
    self.opened(store).map_or(RunState::Idle, |store| store.run_state())
  }

  // A store already open on this device, the system's included.
  pub(crate) fn opened(&self, id: &str) -> Option<Rc<ServiceStore>> {
    if id == self.system.id {
      return Some(Rc::clone(&self.system));
    }
    self.open.stores.borrow().get(id).cloned()
  }

  // The stores open on this device, the system's aside.
  pub(crate) fn open_stores(&self) -> Vec<Rc<ServiceStore>> {
    self.open.stores.borrow().values().cloned().collect()
  }
}

// A store's pointer to an op.
pub(crate) fn pointer_to(row: &LogRow) -> Pointer {
  Pointer { hash: row.op.hash.clone(), time: row.op.time, device: row.op.device.clone() }
}

// Remove a database file and its write-ahead files.
fn remove_files(path: PathBuf) -> Result<()> {
  for suffix in ["", "-wal", "-shm"] {
    let file = PathBuf::from(format!("{}{suffix}", path.display()));
    if file.exists() {
      std::fs::remove_file(&file).map_err(|error| bad_request(format!("Cannot delete {}: {error}", file.display())))?;
    }
  }
  Ok(())
}

// Why an op is refused once the op whose call made it has failed.
pub(crate) fn cause_failed(cause: &str) -> ServiceError {
  ServiceError::new(409, "CAUSE_FAILED", format!("The op whose call made this one, {cause}, failed"))
}
