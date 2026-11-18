//! A service's store on this device: the file holding its tables and the
//! core's record of what it did with each op addressed to it, its pointers
//! into its channel's log and its results; the service that runs in it, the
//! name its channel calls it by and the channel it belongs to; its OpWriter,
//! the callers waiting on its ops, and its RunState.
//!
//! A service's ops apply one at a time, each in its transaction in this file:
//! one store, one writer. Different services apply side by side, though their
//! ops share their channel's log. `[@carbon]` runs in a store of its own in
//! every channel, the system's being the system database.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use serde::Serialize;
use serde_json::Value;

use crate::sql::call_window::CallWindow;
use crate::calls::deadline::Deadline;
use crate::calls::dispatch::{Reply, Settled};
use crate::error::{Result, ServiceError, conflict, internal, not_found};
use crate::channels::channel::Channel;
use crate::ops::op_writer::OpWriter;
use crate::sql::schema::check_keys;
use crate::services::Service;
use crate::store::Store;
use crate::calls::call::address;
use crate::{Call, Db};
use carbondb_service_bindings::migrations::Migration;

// The core's own tables in every service's store, its pointers and results,
// as forward-only migrations. Each name carries its set, since the system's
// store tracks the system database's migrations beside these.
pub const MIGRATIONS: &[Migration] = &[("store/0001_pointers_and_results", include_str!("../../migrations/store/0001_pointers_and_results.sql"))];

// Where a space is in applying its ops: initializing until its package is
// here and compiled, set up and healthy (`crate::channels::lifecycle`), then idle, or
// applying one; or stopped, with why, until it is resumed. Its ops wait in
// every state but idle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum RunState {
  Initializing { services: Vec<String> },
  Idle,
  Applying { op: String },
  Stopped { error: ServiceError },
}

// What a store runs: the package registered on this device that answers its
// calls, and the name its channel calls it by, `@carbon` for `[@carbon]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Runs {
  pub package: String,
  pub name: String,
}

pub struct ServiceStore {
  pub id: String,
  // The channel it belongs to, whose log holds its ops.
  pub channel: Rc<Channel>,
  store: Store,
  pub runs: Runs,
  // Its service, once set up here; until then its ops wait.
  service: RefCell<Option<Rc<Service>>>,
  pub(crate) op_writer: OpWriter,
  // The callers waiting on an op's result, by op hash, and the deadline each
  // was taken under. An op resumed from the log has none.
  waiting_ops: RefCell<HashMap<String, Reply>>,
  deadlines: RefCell<HashMap<String, Rc<Deadline>>>,
  // The ops whose callers' deadlines passed before they ran, with the 504
  // their callers got: each is refused with it when its turn comes.
  expired: RefCell<HashMap<String, ServiceError>>,
  // The stores whose ops wait on an op, by op hash, when another service's
  // call posted it: a write it posts back into one of them would wait forever.
  awaiting: RefCell<HashMap<String, Vec<String>>>,
  run_state: RefCell<RunState>,
  // Whether pending ops are being applied further up the stack, since an op
  // can settle while it is being started.
  pub(crate) draining: Cell<bool>,
  // The ops to take up again between turns, their cause's outcome having changed.
  reconsidering: RefCell<Vec<String>>,
}

impl ServiceStore {
  // The system's `[@carbon]` store, over the system database, whose file
  // holds a store's tables and, after them, the system database's own.
  pub fn system(channel: &Rc<Channel>, store: Store, system: &[Migration], carbon: Rc<Service>) -> Result<ServiceStore> {
    let space = ServiceStore::over(channel, &channel.carbon(), store, &[MIGRATIONS, system].concat(), carbon_runs(&carbon))?;
    space.service.replace(Some(carbon));
    Ok(space)
  }

  // A channel's `[@carbon]` store, running `[@carbon]` from the start.
  pub fn carbon(channel: &Rc<Channel>, carbon: Rc<Service>) -> Result<ServiceStore> {
    let id = channel.carbon();
    let space = ServiceStore::over(channel, &id, Store::open(&channel.store_path(&id))?, MIGRATIONS, carbon_runs(&carbon))?;
    space.service.replace(Some(carbon));
    Ok(space)
  }

  // A new, empty store for a service of a channel.
  pub fn create(channel: &Rc<Channel>, id: &str, runs: Runs) -> Result<ServiceStore> {
    let path = channel.store_path(id);
    if path.exists() {
      return Err(conflict(format!("A service's store already exists at {}", path.display())));
    }
    ServiceStore::open(channel, id, runs)
  }

  pub fn open(channel: &Rc<Channel>, id: &str, runs: Runs) -> Result<ServiceStore> {
    ServiceStore::over(channel, id, Store::open(&channel.store_path(id))?, MIGRATIONS, runs)
  }

  fn over(channel: &Rc<Channel>, id: &str, store: Store, migrations: &[Migration], runs: Runs) -> Result<ServiceStore> {
    store.migrate(migrations)?;
    Ok(ServiceStore {
      id: id.to_string(),
      channel: Rc::clone(channel),
      store,
      runs,
      service: RefCell::default(),
      op_writer: OpWriter::default(),
      waiting_ops: RefCell::default(),
      deadlines: RefCell::default(),
      expired: RefCell::default(),
      awaiting: RefCell::default(),
      run_state: RefCell::new(RunState::Idle),
      draining: Cell::new(false),
      reconsidering: RefCell::default(),
    })
  }

  // Take up an op again before the next turn, its cause's outcome having changed.
  pub(crate) fn reconsider(&self, hash: &str) {
    self.reconsidering.borrow_mut().push(hash.to_string());
  }

  // The ops to take up again now, taken.
  pub(crate) fn reconsidering(&self) -> Vec<String> {
    std::mem::take(&mut *self.reconsidering.borrow_mut())
  }

  // Whether it runs `[@carbon]`.
  pub fn is_carbon(&self) -> bool {
    self.runs.name == CARBON
  }

  pub fn store(&self) -> &Store {
    &self.store
  }

  // Its service, once set up here.
  pub fn service(&self) -> Option<Rc<Service>> {
    self.service.borrow().clone()
  }

  // Its service as calls name it, by the name its channel gives it: `[@carbon]`, or `[name]`.
  pub fn called(&self) -> String {
    format!("[{}]", self.runs.name)
  }

  // Whether a call is to this store's own service, by the name it goes by.
  pub fn answers(&self, call: &Call) -> bool {
    address(call) == self.runs.name
  }

  pub fn run_state(&self) -> RunState {
    self.run_state.borrow().clone()
  }

  pub(crate) fn set_run_state(&self, state: RunState) {
    *self.run_state.borrow_mut() = state;
  }

  pub fn is_applying(&self) -> bool {
    matches!(*self.run_state.borrow(), RunState::Applying { .. })
  }

  // Wait for an op's result, posted under `deadline` by a call that the ops
  // of these spaces wait on.
  pub(crate) fn wait_for(&self, op: &str, awaiting: Vec<String>, reply: Reply, deadline: Rc<Deadline>) {
    self.waiting_ops.borrow_mut().insert(op.to_string(), reply);
    self.deadlines.borrow_mut().insert(op.to_string(), deadline);
    if !awaiting.is_empty() {
      self.awaiting.borrow_mut().insert(op.to_string(), awaiting);
    }
  }

  // The deadlines of the ops waiting here for their callers, to begin once the space is ready.
  pub(crate) fn waiting_deadlines(&self) -> Vec<Rc<Deadline>> {
    self.deadlines.borrow().values().cloned().collect()
  }

  // The deadline an op's caller took it under, as the op begins.
  pub(crate) fn deadline_of(&self, op: &str) -> Option<Rc<Deadline>> {
    self.deadlines.borrow_mut().remove(op)
  }

  // An op's caller's deadline passed before it ran: answer the caller, and
  // refuse the op with the same failure when its turn comes.
  pub(crate) fn give_up(&self, op: &str, error: ServiceError) {
    self.deadlines.borrow_mut().remove(op);
    let waiting = self.waiting_ops.borrow_mut().remove(op);
    if let Some(reply) = waiting {
      self.expired.borrow_mut().insert(op.to_string(), error.clone());
      reply(Err(error))
    }
  }

  // Why an op is refused before it runs, when its caller gave up on it.
  pub(crate) fn expired(&self, op: &str) -> Option<ServiceError> {
    self.expired.borrow_mut().remove(op)
  }

  // The spaces whose ops wait on an op, as it begins.
  pub(crate) fn awaiting(&self, op: &str) -> Vec<String> {
    self.awaiting.borrow_mut().remove(op).unwrap_or_default()
  }

  // Answer whoever waits on an op's result.
  pub(crate) fn answer(&self, op: &str, settled: Result<Settled>) {
    self.deadlines.borrow_mut().remove(op);
    let waiting = self.waiting_ops.borrow_mut().remove(op);
    if let Some(reply) = waiting {
      reply(settled)
    }
  }

  // Answer everyone waiting on this space's ops with the same failure.
  pub(crate) fn answer_all(&self, error: &ServiceError) {
    self.deadlines.borrow_mut().clear();
    let waiting = std::mem::take(&mut *self.waiting_ops.borrow_mut());
    for reply in waiting.into_values() {
      reply(Err(error.clone()))
    }
  }

  // Set up its service here: its `setup` in a transaction of its own, outside
  // any op, then the check every table's keys must pass. A space is set up
  // only while it initializes, so no op is applying.
  pub fn set_up(self: &Rc<Self>, service: &Rc<Service>) -> Result<()> {
    self.store.write(|store| {
      let binding = Rc::new(DbBinding { space: Rc::clone(self), window: CallWindow { service: true, op: None }, open: Cell::new(true) });
      let initialized = (service.setup)(Rc::clone(&binding) as Rc<dyn Db>);
      binding.open.set(false);
      initialized?;
      check_keys(store)
    })?;
    self.service.replace(Some(Rc::clone(service)));
    Ok(())
  }
}

// The `db` binding a service's `setup` receives: its own tables in this
// space, open only while `setup` runs.
struct DbBinding {
  space: Rc<ServiceStore>,
  window: CallWindow,
  open: Cell<bool>,
}

impl DbBinding {
  fn scoped(&self) -> Result<crate::sql::call_window::Scoped<'_>> {
    if !self.open.get() {
      return Err(internal("A service's setup has returned, so its db binding is closed"));
    }
    Ok(self.space.store.scoped(self.window.clone()))
  }
}

impl Db for DbBinding {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    self.scoped()?.run(sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    self.scoped()?.all(sql, params)
  }
}

// The name `[@carbon]` goes by in every channel.
pub const CARBON: &str = "@carbon";

fn carbon_runs(carbon: &Service) -> Runs {
  Runs { package: carbon.name.clone(), name: CARBON.into() }
}

// Where a store id leads when this device does not hold it.
pub fn missing(id: &str) -> crate::error::ServiceError {
  not_found(format!("No service {id} on this device"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Routing;
  use crate::channels::channel::Kind;
  use serde_json::json;

  fn notes() -> Runs {
    Runs { package: "notes".into(), name: "notes".into() }
  }

  fn channel(directory: &tempfile::TempDir) -> Rc<Channel> {
    Rc::new(Channel::open(directory.path(), "room", None, Kind::Replicated).unwrap())
  }

  fn space() -> (tempfile::TempDir, Rc<ServiceStore>) {
    let directory = tempfile::tempdir().unwrap();
    let space = Rc::new(ServiceStore::create(&channel(&directory), "s", notes()).unwrap());
    (directory, space)
  }

  fn service(name: &str, schema: &'static str) -> Rc<Service> {
    Rc::new(Service::replicated(name, "v1", Some("db"), crate::services::DEFAULT_TIMEOUT, Rc::new(move |db: Rc<dyn Db>| db.run(schema, &[]).map(|_| ())), Rc::new(|_| {})))
  }

  #[test]
  fn names_its_service_as_calls_name_it() {
    let (directory, space) = space();
    assert_eq!(space.called(), "[notes]");
    let carbon = ServiceStore::carbon(&channel(&directory), Rc::new(crate::services::carbon::service())).unwrap();
    assert_eq!(carbon.called(), "[@carbon]");
    assert!(carbon.answers(&Call::from_key("GET [@carbon]/status").unwrap()));
  }

  // The core migrations a store's file has applied, in order.
  fn applied(store: &Store) -> Vec<String> {
    store.all("SELECT name FROM carbon_migrations ORDER BY name", &[]).unwrap().iter().map(|row| row["name"].as_str().unwrap().to_string()).collect()
  }

  fn names(migrations: &[Migration]) -> Vec<String> {
    migrations.iter().map(|(name, _)| name.to_string()).collect()
  }

  #[test]
  fn refuses_a_store_file_migrated_by_newer_code() {
    let directory = tempfile::tempdir().unwrap();
    let space = ServiceStore::open(&channel(&directory), "s", notes()).unwrap();
    space.store().run("INSERT INTO carbon_migrations (name) VALUES ('store/9999_from_the_future')", &[]).unwrap();
    drop(space);
    assert_eq!(ServiceStore::open(&channel(&directory), "s", notes()).err().unwrap().status, 409);
  }

  #[test]
  fn the_system_store_holds_a_stores_tables_and_the_system_tables_after_them() {
    let directory = tempfile::tempdir().unwrap();
    let system: &[Migration] = &[("system/0001_things", "CREATE TABLE carbon_system_thing (id TEXT PRIMARY KEY)")];
    let root = ServiceStore::system(&channel(&directory), Store::open(&directory.path().join("system.db")).unwrap(), system, Rc::new(crate::services::carbon::service())).unwrap();
    assert_eq!(applied(root.store()), [names(MIGRATIONS), names(system)].concat());
    assert!(root.answers(&Call::from_key("GET [@carbon]/identities").unwrap()));
  }

  #[test]
  fn creates_a_store_once() {
    let (directory, _space) = space();
    assert_eq!(ServiceStore::create(&channel(&directory), "s", notes()).err().unwrap().status, 409);
  }

  #[test]
  fn sets_up_its_one_service_through_its_init_under_plain_names() {
    let (_directory, space) = space();
    assert_eq!(space.service().map(|service| service.name.clone()), None);
    let notes = service("notes", "CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID");
    space.set_up(&notes).unwrap();
    space.set_up(&notes).unwrap();
    assert_eq!(space.service().map(|service| service.name.clone()), Some("notes".into()));
    assert_eq!(space.store().all("SELECT id, text FROM note", &[]).unwrap(), Vec::<Value>::new());
    let answered: Vec<bool> = ["GET [notes]/texts", "GET [chat]/messages", "GET [@notes]/texts"].iter().map(|key| space.answers(&Call::from_key(key).unwrap())).collect();
    assert_eq!(answered, [true, false, false]);
  }

  #[test]
  fn refuses_a_service_whose_keys_sqlite_assigns_and_keeps_none_of_its_schema() {
    let (_directory, space) = space();
    let error = space.set_up(&service("notes", "CREATE TABLE note (id INTEGER PRIMARY KEY, text TEXT)")).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"table": "note"}))));
    assert!(space.store().get("SELECT name FROM sqlite_schema WHERE name = 'note'", &[]).unwrap().is_null());
    assert_eq!(space.service().map(|service| service.name.clone()), None);
  }
}
