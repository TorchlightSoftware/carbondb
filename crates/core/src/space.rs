//! A space on this device, as the host keeps it: its database, the services
//! provisioned there, its RunQueue, and who listens for its commits. The system
//! database is the root space: local, never replicated, and answering only
//! local services.
//!
//! What a space is (its origin, its refs) is `[space]`'s business; the host
//! keeps only which services it has provisioned here, and how far.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use serde_json::json;

use crate::dispatch::Job;
use crate::error::{Result, bad_request, conflict, not_found};
use crate::oplog;
use crate::run_queue::RunQueue;
use crate::services::Service;
use crate::store::Store;
use crate::Db;

// What the core keeps in every space for itself, beside the op log.
const HOST_SCHEMA: &str = include_str!("../migrations/space.sql");

// Called with the tables each commit wrote.
pub type Listener = Rc<dyn Fn(&[String])>;

pub struct Space {
  pub id: String,
  root: bool,
  store: Store,
  // The replicated services provisioned here, which are the ones it runs.
  provisioned: RefCell<Vec<String>>,
  pub(crate) queue: RefCell<RunQueue<Job>>,
  listeners: RefCell<Vec<Listener>>,
}

impl Space {
  // The root space over the system database.
  pub fn root(store: Store) -> Result<Space> {
    store.run_batch(oplog::SCHEMA)?;
    Ok(Space::over("system", true, store))
  }

  // A new, empty space's database, ready for its origin op.
  pub fn create(path: &Path, id: &str) -> Result<Space> {
    if path.exists() {
      return Err(conflict(format!("A space already exists at {}", path.display())));
    }
    Space::open(path, id)
  }

  pub fn open(path: &Path, id: &str) -> Result<Space> {
    let store = Store::open(path)?;
    store.run_batch(HOST_SCHEMA)?;
    store.run_batch(oplog::SCHEMA)?;
    Ok(Space::over(id, false, store))
  }

  fn over(id: &str, root: bool, store: Store) -> Space {
    Space { id: id.to_string(), root, store, provisioned: RefCell::default(), queue: RefCell::default(), listeners: RefCell::default() }
  }

  pub fn is_root(&self) -> bool {
    self.root
  }

  pub fn store(&self) -> &Store {
    &self.store
  }

  // Whether a replicated service runs here: one this space has provisioned.
  pub fn runs(&self, service: &str) -> bool {
    self.provisioned.borrow().iter().any(|name| name == service)
  }

  // The services provisioned here, as the host recorded them, to provision
  // again on open.
  pub fn provisioned(&self) -> Result<Vec<String>> {
    let rows = self.store.all("SELECT service FROM carbon_migration ORDER BY service", &[])?;
    Ok(rows.iter().filter_map(|row| row["service"].as_str().map(str::to_string)).collect())
  }

  // Provision a service: run the migrations it has not run here yet. Done when
  // the op that names it applies, and again on every open, before any call.
  pub fn provision(&self, service: &Service) -> Result<()> {
    migrate(&self.store, &service.name, &service.migrations)?;
    if !self.runs(&service.name) {
      self.provisioned.borrow_mut().push(service.name.clone());
    }
    Ok(())
  }

  pub fn listen(&self, listener: Listener) {
    self.listeners.borrow_mut().push(listener);
  }

  // Tell every listener what a commit wrote.
  pub(crate) fn committed(&self, tables: &[String]) {
    let listeners = self.listeners.borrow().clone();
    for listener in listeners {
      listener(tables);
    }
  }
}

// Run a service's migrations that this space has not run yet. Every table must
// keep a primary key: sessions record no changes to a table without one, so
// undo would silently miss it.
pub fn migrate(store: &Store, service: &str, migrations: &[impl AsRef<str>]) -> Result<()> {
  store.write(|store| {
    let done = store.get("SELECT count FROM carbon_migration WHERE service = ?", &[json!(service)])?["count"].as_u64().unwrap_or(0) as usize;
    if done > migrations.len() {
      return Err(conflict(format!("Service '{service}' has run {done} migrations here, more than the {} it declares", migrations.len())));
    }
    for migration in &migrations[done..] {
      store.run_batch(migration.as_ref())?;
    }
    require_primary_keys(store)?;
    store.run(
      "INSERT INTO carbon_migration (service, count) VALUES (?, ?) ON CONFLICT (service) DO UPDATE SET count = excluded.count",
      &[json!(service), json!(migrations.len())],
    )?;
    Ok(())
  })?;
  Ok(())
}

fn require_primary_keys(store: &Store) -> Result<()> {
  let tables = store.all("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'", &[])?;
  for table in tables.iter().filter_map(|row| row["name"].as_str()) {
    if store.get("SELECT 1 AS keyed FROM pragma_table_info(?) WHERE pk > 0", &[json!(table)])?.is_null() {
      return Err(bad_request(format!("Table '{table}' has no primary key; every table needs one")));
    }
  }
  Ok(())
}

// Where a space id leads when this device does not hold it.
pub fn missing(id: &str) -> crate::error::ServiceError {
  not_found(format!("No space {id} on this device"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn space() -> (tempfile::TempDir, Space) {
    let directory = tempfile::tempdir().unwrap();
    let space = Space::create(&directory.path().join("s.db"), "s").unwrap();
    (directory, space)
  }

  fn service(name: &str, migrations: &[&str]) -> Service {
    Service::replicated(name, migrations.iter().map(|sql| sql.to_string()).collect(), Rc::new(|_| {}))
  }

  #[test]
  fn creates_a_space_once() {
    let (directory, _space) = space();
    assert_eq!(Space::create(&directory.path().join("s.db"), "s").err().unwrap().status, 409);
  }

  #[test]
  fn runs_only_what_it_has_provisioned_and_migrates_once() {
    let (_directory, space) = space();
    assert!(!space.runs("chat"));
    let first = service("chat", &["CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT)"]);
    space.provision(&first).unwrap();
    space.provision(&first).unwrap();
    assert!(space.runs("chat"));
    assert_eq!(space.provisioned().unwrap(), vec!["chat"]);
    let second = service("chat", &["CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT)", "ALTER TABLE message ADD COLUMN edited_at INTEGER"]);
    space.provision(&second).unwrap();
    assert_eq!(space.store().all("SELECT id, text, edited_at FROM message", &[]).unwrap(), Vec::<serde_json::Value>::new());
  }

  #[test]
  fn refuses_a_table_without_a_primary_key() {
    let (_directory, space) = space();
    assert_eq!(space.provision(&service("notes", &["CREATE TABLE note (text TEXT)"])).unwrap_err().status, 400);
    assert!(space.store().get("SELECT name FROM sqlite_schema WHERE name = 'note'", &[]).unwrap().is_null());
  }
}
