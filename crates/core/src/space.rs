//! A space on this device, as the host keeps it: its database, the one
//! service that runs in it, the name its channel calls that service by and
//! the channel it belongs to, its OpWriter, the callers waiting on its ops,
//! and its RunState. The system database is the root space: local, never
//! replicated, and running only `[@carbon]`.
//!
//! A space's file holds its service's tables beside the core's own record of
//! it (the op log, results and migrations, named `carbon_`). Its ops apply one
//! at a time: one store, one writer. Different spaces apply side by side.
//!
//! Which service a space runs, and in which channel, is this device's own
//! bookkeeping, kept in the system database when the space is created here;
//! a space keeps no record about itself.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use serde::Serialize;
use serde_json::Value;

use crate::call_window::CallWindow;
use crate::dispatch::{Reply, Settled};
use crate::error::{Result, ServiceError, conflict, internal, not_found};
use crate::ops::op_writer::OpWriter;
use crate::sql::schema::check_keys;
use crate::services::Service;
use crate::store::Store;
use crate::{Call, Db};
use carbondb_service_bindings::migrations::Migration;

// The core's own tables in every space's file, the op log and results, as
// forward-only migrations. Each name carries its set, since the root space's
// file tracks the system database's migrations beside these.
pub const MIGRATIONS: &[Migration] = &[
  ("space/0001_oplog_and_results", include_str!("../migrations/space/0001_oplog_and_results.sql")),
  ("space/0002_sub_calls", include_str!("../migrations/space/0002_sub_calls.sql")),
  ("space/0003_read_sets_and_changesets", include_str!("../migrations/space/0003_read_sets_and_changesets.sql")),
  ("space/0004_effects", include_str!("../migrations/space/0004_effects.sql")),
];

// Where a space is in applying its ops: initializing until its service is
// registered and set up here, then idle, or applying one; or stopped by a
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

// What a space runs: the package registered on this device that answers its
// calls, the name its channel calls it by, and the channel it belongs to. A
// channel's own space belongs to itself; the system space to none.
#[derive(Debug, Clone, PartialEq)]
pub struct Runs {
  pub package: String,
  pub name: String,
  pub channel: Option<String>,
}

pub struct Space {
  pub id: String,
  root: bool,
  store: Store,
  pub runs: Runs,
  // Its service, once registered and set up here; until then its ops wait.
  service: RefCell<Option<Rc<Service>>>,
  // Its service, registered while an op applied, to set up once that op is done.
  awaiting: RefCell<Option<Rc<Service>>>,
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
  // tables and, after them, the system database's own. It runs `[@carbon]`.
  pub fn root(store: Store, system: &[Migration], carbon: Rc<Service>) -> Result<Space> {
    let runs = Runs { package: carbon.name.clone(), name: carbon.name.clone(), channel: None };
    let space = Space::over("system", true, store, &[MIGRATIONS, system].concat(), runs)?;
    space.service.replace(Some(carbon));
    Ok(space)
  }

  // A new, empty space's database.
  pub fn create(path: &Path, id: &str, runs: Runs) -> Result<Space> {
    if path.exists() {
      return Err(conflict(format!("A space already exists at {}", path.display())));
    }
    Space::open(path, id, runs)
  }

  pub fn open(path: &Path, id: &str, runs: Runs) -> Result<Space> {
    Space::over(id, false, Store::open(path)?, MIGRATIONS, runs)
  }

  fn over(id: &str, root: bool, store: Store, migrations: &[Migration], runs: Runs) -> Result<Space> {
    store.migrate(migrations)?;
    Ok(Space {
      id: id.to_string(),
      root,
      store,
      runs,
      service: RefCell::default(),
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

  // Its service, once set up here.
  pub fn service(&self) -> Option<Rc<Service>> {
    self.service.borrow().clone()
  }

  // Whether a call is to this space's own service, by the name it goes by:
  // `[@carbon]` in the system space, its channel name anywhere else.
  pub fn answers(&self, call: &Call) -> bool {
    call.confined == self.root && call.service == self.runs.name
  }

  // The service this space waits for, until it is set up here.
  pub fn not_set_up(&self) -> Option<String> {
    self.service.borrow().is_none().then(|| self.runs.package.clone())
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

  // Set up its service here: its `init` in a transaction of its own, outside
  // any op, then the check every table's keys must pass. While an op is
  // applying, it waits until that op is done.
  pub fn set_up(self: &Rc<Self>, service: &Rc<Service>) -> Result<()> {
    if self.is_applying() {
      self.awaiting.replace(Some(Rc::clone(service)));
      return Ok(());
    }
    self.store.write(|store| {
      let binding = Rc::new(DbBinding { space: Rc::clone(self), window: CallWindow { service: true, op: None }, open: Cell::new(true) });
      let initialized = (service.init)(Rc::clone(&binding) as Rc<dyn Db>);
      binding.open.set(false);
      initialized?;
      check_keys(store)
    })?;
    self.service.replace(Some(Rc::clone(service)));
    Ok(())
  }

  // Set up the service that was registered while an op applied.
  pub(crate) fn set_up_awaiting(self: &Rc<Self>) -> Result<()> {
    let awaiting = self.awaiting.take();
    awaiting.iter().try_for_each(|service| self.set_up(service))
  }
}

// The `db` binding a service's `init` receives: its own tables in this
// space, open only while `init` runs.
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
  use crate::Routing;
  use serde_json::json;

  fn notes() -> Runs {
    Runs { package: "notes".into(), name: "notes".into(), channel: Some("room".into()) }
  }

  fn space() -> (tempfile::TempDir, Rc<Space>) {
    let directory = tempfile::tempdir().unwrap();
    let space = Rc::new(Space::create(&directory.path().join("s.db"), "s", notes()).unwrap());
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
  fn brings_a_space_file_from_before_effects_up_to_date_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("s.db");
    Store::open(&path).unwrap().migrate(&MIGRATIONS[..3]).unwrap();
    for opening in ["first", "again"] {
      let space = Space::open(&path, "s", notes()).unwrap();
      assert_eq!(applied(space.store()), names(MIGRATIONS), "{opening}");
      assert!(space.store().all("SELECT effects FROM carbon_oplog", &[]).is_ok(), "{opening}");
    }
  }

  #[test]
  fn refuses_a_space_file_migrated_by_newer_code() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("s.db");
    let space = Space::open(&path, "s", notes()).unwrap();
    space.store().run("INSERT INTO carbon_migrations (name) VALUES ('space/9999_from_the_future')", &[]).unwrap();
    drop(space);
    assert_eq!(Space::open(&path, "s", notes()).err().unwrap().status, 409);
  }

  #[test]
  fn the_root_space_holds_the_space_tables_and_the_system_tables_after_them() {
    let directory = tempfile::tempdir().unwrap();
    let system: &[Migration] = &[("system/0001_things", "CREATE TABLE carbon_system_thing (id TEXT PRIMARY KEY)")];
    let root = Space::root(Store::open(&directory.path().join("system.db")).unwrap(), system, Rc::new(crate::services::carbon::service())).unwrap();
    assert_eq!(applied(root.store()), [names(MIGRATIONS), names(system)].concat());
    assert!(root.answers(&Call::from_key("GET [@carbon]/identities").unwrap()));
  }

  #[test]
  fn creates_a_space_once() {
    let (directory, _space) = space();
    assert_eq!(Space::create(&directory.path().join("s.db"), "s", notes()).err().unwrap().status, 409);
  }

  #[test]
  fn sets_up_its_one_service_through_its_init_under_plain_names() {
    let (_directory, space) = space();
    assert_eq!(space.not_set_up(), Some("notes".into()));
    let notes = service("notes", "CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID");
    space.set_up(&notes).unwrap();
    space.set_up(&notes).unwrap();
    assert_eq!(space.not_set_up(), None);
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
    assert_eq!(space.not_set_up(), Some("notes".into()));
  }
}
