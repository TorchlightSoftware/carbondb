//! A space on this device, as the host keeps it: its database, the services
//! its origin names and the ones set up here, its OpWriter, the callers
//! waiting on its ops, and its RunState. The system database is the root
//! space: local, never replicated, and answering only local services.
//!
//! Its ops apply one at a time: one store, one writer. Different spaces apply
//! side by side.
//!
//! What a space is (its origin, its refs) is `[space]`'s business. The host
//! keeps no record of its own: it learns which services a space runs by asking
//! `[space]`, and each service keeps its own schema through its `init`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use serde::Serialize;
use serde_json::Value;

use crate::call_window::CallWindow;
use crate::dispatch::{Reply, Settled};
use crate::error::{Result, ServiceError, conflict, internal, not_found};
use crate::op_writer::OpWriter;
use crate::schema::check_keys;
use crate::services::Service;
use crate::store::Store;
use crate::Db;
use carbondb_service_bindings::migrations::Migration;

// The core's own tables in every space's file, the op log and results, as
// forward-only migrations. Each name carries its set, since the root space's
// file tracks the system database's migrations beside these.
pub const MIGRATIONS: &[Migration] = &[
  ("space/0001_oplog_and_results", include_str!("../migrations/space/0001_oplog_and_results.sql")),
  ("space/0002_sub_calls", include_str!("../migrations/space/0002_sub_calls.sql")),
];

// Where a space is in applying its ops: initializing until every service its
// origin names is set up here, then idle, or applying one; or stopped by a
// system failure until it is fixed by hand and resumed. Its ops wait in every
// state but idle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum RunState {
  Initializing { services: Vec<String> },
  Idle,
  Applying { op: String },
  Stopped { error: ServiceError },
}

pub struct Space {
  pub id: String,
  root: bool,
  store: Store,
  // The services its origin names, which a device must run to answer them here.
  named: RefCell<Vec<String>>,
  // The services set up here, which are the ones it answers.
  running: RefCell<Vec<String>>,
  // Services waiting to be set up until the op now applying is done.
  awaiting: RefCell<Vec<Rc<Service>>>,
  pub(crate) op_writer: OpWriter,
  // The callers waiting on an op's result, by op hash. An op resumed from the
  // log has none.
  waiting_ops: RefCell<HashMap<String, Reply>>,
  run_state: RefCell<RunState>,
  // Whether pending ops are being applied further up the stack, since an op
  // can settle while it is being started.
  pub(crate) draining: Cell<bool>,
}

impl Space {
  // The root space over the system database, whose file holds a space's
  // tables and, after them, the system database's own.
  pub fn root(store: Store, system: &[Migration]) -> Result<Space> {
    Space::over("system", true, store, &[MIGRATIONS, system].concat())
  }

  // A new, empty space's database, ready for its origin op.
  pub fn create(path: &Path, id: &str) -> Result<Space> {
    if path.exists() {
      return Err(conflict(format!("A space already exists at {}", path.display())));
    }
    Space::open(path, id)
  }

  pub fn open(path: &Path, id: &str) -> Result<Space> {
    Space::over(id, false, Store::open(path)?, MIGRATIONS)
  }

  fn over(id: &str, root: bool, store: Store, migrations: &[Migration]) -> Result<Space> {
    store.migrate(migrations)?;
    Ok(Space {
      id: id.to_string(),
      root,
      store,
      named: RefCell::default(),
      running: RefCell::default(),
      awaiting: RefCell::default(),
      op_writer: OpWriter::default(),
      waiting_ops: RefCell::default(),
      run_state: RefCell::new(RunState::Idle),
      draining: Cell::new(false),
    })
  }

  pub fn is_root(&self) -> bool {
    self.root
  }

  pub fn store(&self) -> &Store {
    &self.store
  }

  // Whether a replicated service answers here: one set up here.
  pub fn runs(&self, service: &str) -> bool {
    self.running.borrow().iter().any(|name| name == service)
  }

  pub fn names(&self, service: &str) -> bool {
    self.named.borrow().iter().any(|name| name == service)
  }

  // The services its origin names, once the host has learned them.
  pub fn name(&self, services: &[String]) {
    *self.named.borrow_mut() = services.to_vec();
  }

  // The services its origin names that are not set up here yet, which hold its ops back.
  pub fn not_set_up(&self) -> Vec<String> {
    self.named.borrow().iter().filter(|name| !self.runs(name)).cloned().collect()
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

  pub(crate) fn wait_for(&self, op: &str, reply: Reply) {
    self.waiting_ops.borrow_mut().insert(op.to_string(), reply);
  }

  // Answer whoever waits on an op's result.
  pub(crate) fn answer(&self, op: &str, settled: Result<Settled>) {
    let waiting = self.waiting_ops.borrow_mut().remove(op);
    if let Some(reply) = waiting {
      reply(settled)
    }
  }

  // Answer everyone waiting on this space's ops with the same failure.
  pub(crate) fn answer_all(&self, error: &ServiceError) {
    let waiting = std::mem::take(&mut *self.waiting_ops.borrow_mut());
    for reply in waiting.into_values() {
      reply(Err(error.clone()))
    }
  }

  // Set up a service here: its `init` in a transaction of its own, outside any
  // op, then the check every table's keys must pass. While an op is applying,
  // it waits until that op is done.
  pub fn set_up(self: &Rc<Self>, service: &Rc<Service>) -> Result<()> {
    if self.is_applying() {
      self.awaiting.borrow_mut().push(Rc::clone(service));
      return Ok(());
    }
    let prefix = service.prefix()?;
    self.store.write(|store| {
      let binding = Rc::new(DbBinding { space: Rc::clone(self), window: CallWindow { prefix: prefix.clone(), op: None }, open: Cell::new(true) });
      let initialized = (service.init)(Rc::clone(&binding) as Rc<dyn Db>);
      binding.open.set(false);
      initialized?;
      prefix.map_or(Ok(()), |prefix| check_keys(store, &prefix))
    })?;
    if !self.runs(&service.name) {
      self.running.borrow_mut().push(service.name.clone());
    }
    Ok(())
  }

  // Set up the services that waited for the op that was applying.
  pub(crate) fn set_up_awaiting(self: &Rc<Self>) -> Result<()> {
    let awaiting = std::mem::take(&mut *self.awaiting.borrow_mut());
    awaiting.iter().try_for_each(|service| self.set_up(service))
  }
}

// The `db` binding a service's `init` receives: its namespace in this space,
// open only while `init` runs.
struct DbBinding {
  space: Rc<Space>,
  window: CallWindow,
  open: Cell<bool>,
}

impl DbBinding {
  fn scoped(&self) -> Result<crate::call_window::Scoped<'_>> {
    if !self.open.get() {
      return Err(internal("A service's init has returned, so its db binding is closed"));
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

// Where a space id leads when this device does not hold it.
pub fn missing(id: &str) -> crate::error::ServiceError {
  not_found(format!("No space {id} on this device"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn space() -> (tempfile::TempDir, Rc<Space>) {
    let directory = tempfile::tempdir().unwrap();
    let space = Rc::new(Space::create(&directory.path().join("s.db"), "s").unwrap());
    (directory, space)
  }

  fn service(name: &str, schema: &'static str) -> Rc<Service> {
    Rc::new(Service::replicated(name, "v1", Some("db"), crate::services::DEFAULT_TIMEOUT, Rc::new(move |db: Rc<dyn Db>| db.run(schema, &[]).map(|_| ())), Rc::new(|_| {})))
  }

  // The core migrations a store's file has applied, in order.
  fn applied(store: &Store) -> Vec<String> {
    store.all("SELECT name FROM carbon_migrations ORDER BY name", &[]).unwrap().iter().map(|row| row["name"].as_str().unwrap().to_string()).collect()
  }

  fn names(migrations: &[Migration]) -> Vec<String> {
    migrations.iter().map(|(name, _)| name.to_string()).collect()
  }

  #[test]
  fn brings_a_space_file_from_before_sub_calls_up_to_date_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("s.db");
    Store::open(&path).unwrap().run_batch(MIGRATIONS[0].1).unwrap();
    for opening in ["first", "again"] {
      let space = Space::open(&path, "s").unwrap();
      assert_eq!(applied(space.store()), names(MIGRATIONS), "{opening}");
      assert!(space.store().all("SELECT sub_calls FROM carbon_oplog", &[]).is_ok(), "{opening}");
    }
  }

  #[test]
  fn refuses_a_space_file_migrated_by_newer_code() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("s.db");
    let space = Space::open(&path, "s").unwrap();
    space.store().run("INSERT INTO carbon_migrations (name) VALUES ('space/9999_from_the_future')", &[]).unwrap();
    drop(space);
    assert_eq!(Space::open(&path, "s").err().unwrap().status, 409);
  }

  #[test]
  fn the_root_space_holds_the_space_tables_and_the_system_tables_after_them() {
    let directory = tempfile::tempdir().unwrap();
    let system: &[Migration] = &[("system/0001_things", "CREATE TABLE carbon_system_thing (id TEXT PRIMARY KEY)")];
    let root = Space::root(Store::open(&directory.path().join("system.db")).unwrap(), system).unwrap();
    assert_eq!(applied(root.store()), [names(MIGRATIONS), names(system)].concat());
  }

  #[test]
  fn creates_a_space_once() {
    let (directory, _space) = space();
    assert_eq!(Space::create(&directory.path().join("s.db"), "s").err().unwrap().status, 409);
  }

  #[test]
  fn sets_up_a_service_through_its_init_in_its_own_namespace() {
    let (_directory, space) = space();
    space.name(&["chat".into()]);
    assert_eq!(space.not_set_up(), ["chat"]);
    let chat = service("chat", "CREATE TABLE IF NOT EXISTS message (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID");
    space.set_up(&chat).unwrap();
    space.set_up(&chat).unwrap();
    assert!(space.runs("chat") && space.not_set_up().is_empty());
    assert_eq!(space.store().all("SELECT id, text FROM \"chat/db/message\"", &[]).unwrap(), Vec::<Value>::new());
  }

  #[test]
  fn refuses_a_service_whose_keys_sqlite_assigns_and_keeps_none_of_its_schema() {
    let (_directory, space) = space();
    let error = space.set_up(&service("notes", "CREATE TABLE note (id INTEGER PRIMARY KEY, text TEXT)")).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"table": "note"}))));
    assert!(space.store().get("SELECT name FROM sqlite_schema WHERE name LIKE 'notes/%'", &[]).unwrap().is_null());
    assert!(!space.runs("notes"));
  }
}
