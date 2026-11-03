//! What CarbonDB itself records in a space's database, beside the app's
//! tables: the origin, refs, an identity's origin, its devices, and how far
//! each service's migrations have run. Every table is prefixed `carbon_`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Result, bad_request, conflict, forbidden, not_found};
use crate::store::{Side, Store};

const SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS carbon_origin (
    one INTEGER PRIMARY KEY CHECK (one = 1),
    kind TEXT NOT NULL, name TEXT NOT NULL, services TEXT NOT NULL, manage TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS carbon_ref (
    space TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, slug TEXT NOT NULL, cascade TEXT NOT NULL,
    UNIQUE (kind, slug)
  );
  CREATE TABLE IF NOT EXISTS carbon_identity (id TEXT PRIMARY KEY, device TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS carbon_device (pubkey TEXT PRIMARY KEY);
  CREATE TABLE IF NOT EXISTS carbon_device_ref (pubkey TEXT PRIMARY KEY, name TEXT NOT NULL, tags TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS carbon_migration (service TEXT PRIMARY KEY, count INTEGER NOT NULL);
";

// The services CarbonDB itself runs, which an app never registers.
pub const BUILTIN_SERVICES: [&str; 3] = ["space", "identity", "device"];

// What a space is, fixed by its first op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
  pub kind: String,
  pub name: String,
  pub services: Vec<String>,
  pub manage: Vec<String>,
}

// A pointer from this space to another. It grants nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
  pub space: String,
  pub kind: String,
  pub name: String,
  pub slug: String,
  #[serde(default)]
  pub cascade: Vec<String>,
}

// A device's private description, kept in the personal space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRef {
  pub pubkey: String,
  pub name: String,
  #[serde(default)]
  pub tags: Vec<String>,
}

// Create a space's database, writing its origin and whatever else its creator
// writes in the same transaction.
pub fn create(path: &Path, origin: &Origin, setup: impl FnOnce(&Store) -> Result<()>) -> Result<Store> {
  if path.exists() {
    return Err(conflict(format!("A space already exists at {}", path.display())));
  }
  let store = Store::open(path)?;
  store.run_batch(SCHEMA)?;
  store.write(|store| {
    write_origin(store, origin)?;
    setup(store)
  })?;
  Ok(store)
}

fn json_column(row: &Value, column: &str) -> Result<Vec<String>> {
  Ok(serde_json::from_str(row[column].as_str().unwrap_or("[]"))?)
}

pub fn write_origin(store: &Store, origin: &Origin) -> Result<()> {
  if !store.get(Side::Writer, "SELECT 1 AS present FROM carbon_origin", &[])?.is_null() {
    return Err(conflict("A space's origin is written once"));
  }
  store.run(
    Side::Writer,
    "INSERT INTO carbon_origin (one, kind, name, services, manage) VALUES (1, ?, ?, ?, ?)",
    &[json!(origin.kind), json!(origin.name), json!(origin.services), json!(origin.manage)],
  )?;
  Ok(())
}

pub fn origin(store: &Store, side: Side) -> Result<Origin> {
  let row = store.get(side, "SELECT kind, name, services, manage FROM carbon_origin", &[])?;
  if row.is_null() {
    return Err(not_found(format!("{} has no origin", store.path().display())));
  }
  Ok(Origin {
    kind: row["kind"].as_str().unwrap_or_default().to_string(),
    name: row["name"].as_str().unwrap_or_default().to_string(),
    services: json_column(&row, "services")?,
    manage: json_column(&row, "manage")?,
  })
}

// The identities that manage this space, with `self` resolved to the identity
// whose identity space this is.
pub fn managers(store: &Store, side: Side) -> Result<Vec<String>> {
  let manage = origin(store, side)?.manage;
  let own = identity_id(store, side)?;
  Ok(manage.into_iter().filter_map(|entry| if entry == "self" { own.clone() } else { Some(entry) }).collect())
}

pub fn require_manage(store: &Store, actor: Option<&str>) -> Result<()> {
  let actor = actor.ok_or_else(|| forbidden("Managing a space needs an acting identity"))?;
  if managers(store, Side::Writer)?.iter().any(|manager| manager == actor) {
    Ok(())
  } else {
    Err(forbidden("Only a manager of this space may change it"))
  }
}

// A second ref to the same space replaces the first.
pub fn add_ref(store: &Store, reference: &Ref) -> Result<()> {
  store.run(
    Side::Writer,
    "INSERT INTO carbon_ref (space, kind, name, slug, cascade) VALUES (?, ?, ?, ?, ?)
       ON CONFLICT (space) DO UPDATE SET kind = excluded.kind, name = excluded.name, slug = excluded.slug, cascade = excluded.cascade",
    &[json!(reference.space), json!(reference.kind), json!(reference.name), json!(reference.slug), json!(reference.cascade)],
  )?;
  Ok(())
}

fn to_ref(row: &Value) -> Result<Ref> {
  Ok(Ref {
    space: row["space"].as_str().unwrap_or_default().to_string(),
    kind: row["kind"].as_str().unwrap_or_default().to_string(),
    name: row["name"].as_str().unwrap_or_default().to_string(),
    slug: row["slug"].as_str().unwrap_or_default().to_string(),
    cascade: json_column(row, "cascade")?,
  })
}

pub fn refs(store: &Store, side: Side) -> Result<Vec<Ref>> {
  store.all(side, "SELECT space, kind, name, slug, cascade FROM carbon_ref ORDER BY kind, slug", &[])?.iter().map(to_ref).collect()
}

pub fn find_ref(store: &Store, side: Side, kind: &str, slug: &str) -> Result<Option<Ref>> {
  let row = store.get(side, "SELECT space, kind, name, slug, cascade FROM carbon_ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)])?;
  if row.is_null() { Ok(None) } else { to_ref(&row).map(Some) }
}

pub fn write_identity_origin(store: &Store, id: &str, device: &str) -> Result<()> {
  if identity_id(store, Side::Writer)?.is_some() {
    return Err(conflict("An identity's origin is written once"));
  }
  store.run(Side::Writer, "INSERT INTO carbon_identity (id, device) VALUES (?, ?)", &[json!(id), json!(device)])?;
  Ok(())
}

pub fn identity_id(store: &Store, side: Side) -> Result<Option<String>> {
  let row = store.get(side, "SELECT id FROM carbon_identity", &[])?;
  Ok(row["id"].as_str().map(str::to_string))
}

pub fn add_device(store: &Store, pubkey: &str) -> Result<()> {
  if !store.get(Side::Writer, "SELECT 1 AS present FROM carbon_device WHERE pubkey = ?", &[json!(pubkey)])?.is_null() {
    return Err(conflict("A device's origin is written once"));
  }
  store.run(Side::Writer, "INSERT INTO carbon_device (pubkey) VALUES (?)", &[json!(pubkey)])?;
  Ok(())
}

pub fn devices(store: &Store, side: Side) -> Result<Vec<String>> {
  let rows = store.all(side, "SELECT pubkey FROM carbon_device ORDER BY pubkey", &[])?;
  Ok(rows.iter().filter_map(|row| row["pubkey"].as_str().map(str::to_string)).collect())
}

// A second ref for the same device replaces the first.
pub fn put_device_ref(store: &Store, device: &DeviceRef) -> Result<()> {
  store.run(
    Side::Writer,
    "INSERT INTO carbon_device_ref (pubkey, name, tags) VALUES (?, ?, ?)
       ON CONFLICT (pubkey) DO UPDATE SET name = excluded.name, tags = excluded.tags",
    &[json!(device.pubkey), json!(device.name), json!(device.tags)],
  )?;
  Ok(())
}

pub fn device_refs(store: &Store, side: Side) -> Result<Vec<DeviceRef>> {
  store
    .all(side, "SELECT pubkey, name, tags FROM carbon_device_ref ORDER BY name", &[])?
    .iter()
    .map(|row| {
      Ok(DeviceRef {
        pubkey: row["pubkey"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tags: json_column(row, "tags")?,
      })
    })
    .collect()
}

// Run a service's migrations that this space has not run yet. Every table must
// keep a primary key: sessions record no changes to a table without one, so
// undo would silently miss it.
pub fn migrate(store: &Store, service: &str, migrations: &[String]) -> Result<()> {
  store.write(|store| {
    let done = store.get(Side::Writer, "SELECT count FROM carbon_migration WHERE service = ?", &[json!(service)])?["count"].as_u64().unwrap_or(0) as usize;
    if done > migrations.len() {
      return Err(conflict(format!("Service '{service}' has run {done} migrations here, more than the {} it declares", migrations.len())));
    }
    for migration in &migrations[done..] {
      store.run_batch(migration)?;
    }
    require_primary_keys(store)?;
    store.run(
      Side::Writer,
      "INSERT INTO carbon_migration (service, count) VALUES (?, ?) ON CONFLICT (service) DO UPDATE SET count = excluded.count",
      &[json!(service), json!(migrations.len())],
    )?;
    Ok(())
  })?;
  Ok(())
}

fn require_primary_keys(store: &Store) -> Result<()> {
  let tables = store.all(Side::Writer, "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'", &[])?;
  for table in tables.iter().filter_map(|row| row["name"].as_str()) {
    let keyed = store.get(Side::Writer, "SELECT 1 AS keyed FROM pragma_table_info(?) WHERE pk > 0", &[json!(table)])?;
    if keyed.is_null() {
      return Err(bad_request(format!("Table '{table}' has no primary key; every table needs one")));
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn origin_of(manage: &[&str]) -> Origin {
    Origin { kind: "team".into(), name: "Sales".into(), services: vec!["chat".into()], manage: manage.iter().map(|entry| entry.to_string()).collect() }
  }

  fn space(origin: &Origin) -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = create(&directory.path().join("s.db"), origin, |_| Ok(())).unwrap();
    (directory, store)
  }

  #[test]
  fn writes_the_origin_once() {
    let (_directory, store) = space(&origin_of(&["brandon"]));
    assert_eq!(origin(&store, Side::Reader).unwrap(), origin_of(&["brandon"]));
    let second = store.write(|store| write_origin(store, &origin_of(&["sally"])));
    assert_eq!(second.unwrap_err().status, 409);
  }

  #[test]
  fn only_managers_may_manage() {
    let (_directory, store) = space(&origin_of(&["brandon"]));
    assert!(require_manage(&store, Some("brandon")).is_ok());
    for actor in [Some("sally"), None] {
      assert_eq!(require_manage(&store, actor).unwrap_err().status, 403);
    }
  }

  #[test]
  fn self_resolves_to_the_identity_of_an_identity_space() {
    let directory = tempfile::tempdir().unwrap();
    let identity = Origin { kind: "identity".into(), name: "brandon".into(), services: vec![], manage: vec!["self".into()] };
    let store = create(&directory.path().join("i.db"), &identity, |store| write_identity_origin(store, "id-brandon", "key-1")).unwrap();
    assert_eq!(managers(&store, Side::Reader).unwrap(), vec!["id-brandon"]);
  }

  #[test]
  fn a_second_ref_to_the_same_space_replaces_the_first() {
    let (_directory, store) = space(&origin_of(&["brandon"]));
    let first = Ref { space: "s1".into(), kind: "team".into(), name: "Sales".into(), slug: "sales".into(), cascade: vec![] };
    let renamed = Ref { name: "Sales West".into(), slug: "sales-west".into(), ..first.clone() };
    store.write(|store| add_ref(store, &first).and_then(|_| add_ref(store, &renamed))).unwrap();
    assert_eq!(refs(&store, Side::Reader).unwrap(), vec![renamed.clone()]);
    assert_eq!(find_ref(&store, Side::Reader, "team", "sales-west").unwrap(), Some(renamed));
    assert_eq!(find_ref(&store, Side::Reader, "team", "sales").unwrap(), None);
  }

  #[test]
  fn migrations_run_once_and_need_primary_keys() {
    let (_directory, store) = space(&origin_of(&["brandon"]));
    let first = vec!["CREATE TABLE message (id TEXT PRIMARY KEY, text TEXT)".to_string()];
    migrate(&store, "chat", &first).unwrap();
    migrate(&store, "chat", &first).unwrap();
    let second = [first.clone(), vec!["ALTER TABLE message ADD COLUMN edited_at INTEGER".to_string()]].concat();
    migrate(&store, "chat", &second).unwrap();
    assert_eq!(store.all(Side::Reader, "SELECT id, text, edited_at FROM message", &[]).unwrap(), Vec::<Value>::new());

    let keyless = vec!["CREATE TABLE note (text TEXT)".to_string()];
    assert_eq!(migrate(&store, "notes", &keyless).unwrap_err().status, 400);
    assert!(store.get(Side::Reader, "SELECT name FROM sqlite_schema WHERE name = 'note'", &[]).unwrap().is_null());
  }
}
