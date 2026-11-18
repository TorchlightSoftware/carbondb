//! Test helpers shared across the core's modules: a CarbonDB on a fresh
//! directory with the scripted test service `[notes]` (`notes`), an identity
//! on it, and a way to make a call and read its answer at once. The host's
//! timers and tasks run as the test drives them (`tasks`), and the world
//! outside is an HTTP server of the test's own (`outside`).

mod notes;
mod outside;
mod tasks;

pub use notes::*;
pub use outside::*;
pub use tasks::*;

use std::rc::Rc;

use serde_json::{Value, json};

use crate::error::Result;
use crate::{Actor, Call, Carbon, Identity, Routing, ServiceStore, Settled};

// A single-node CarbonDB in a temporary directory, which lives as long as the
// directory, running `[notes]`.
pub fn carbon() -> (tempfile::TempDir, Rc<Carbon>) {
  let (directory, carbon, _timers) = timed();
  (directory, carbon)
}

// A test's config: single-node, its compiled components kept where every
// test's are, so a run compiles each component once.
pub fn config() -> String {
  config_syncing("mode = \"none\"")
}

// A test's config with the `[sync]` section given.
pub fn config_syncing(sync: &str) -> String {
  let compiled = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/carbondb-compiled");
  format!("system = \"data/system.db\"\ncompiled = {:?}\n[sync]\n{sync}\n", compiled.display().to_string())
}

// A CarbonDB that syncs in the `ref` mode, its nodes finding no peers: no
// relay, no bootstrap and no mDNS.
pub fn replicated() -> (tempfile::TempDir, Rc<Carbon>) {
  syncing("mode = \"ref\"\nmdns = \"off\"")
}

// A CarbonDB with the `[sync]` section given, running `[notes]`.
pub fn syncing(sync: &str) -> (tempfile::TempDir, Rc<Carbon>) {
  let (directory, carbon, _timers) = syncing_timed(sync);
  (directory, carbon)
}

// The same, with the timers it schedules on.
pub fn syncing_timed(sync: &str) -> (tempfile::TempDir, Rc<Carbon>, Timers) {
  let directory = tempfile::tempdir().unwrap();
  std::fs::write(directory.path().join("carbondb.toml"), config_syncing(sync)).unwrap();
  let timers = Timers::default();
  let carbon = Carbon::open(&directory.path().join("carbondb.toml"), timers.schedule(), tasks()).unwrap();
  register_notes(&carbon);
  (directory, carbon, timers)
}

// The core's test fixture `[probe]`, a Rust component that reports what it sees of its host.
pub const PROBE: &[u8] = include_bytes!("../../fixtures/probe.wasm");

// A package's bytes with a custom section of its own appended, so its version
// is new and this process compiles it afresh.
pub fn afresh(bytes: &[u8]) -> Vec<u8> {
  static FRESH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
  let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
  let name = format!("carbondb:test:{since}:{}", FRESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
  [bytes, &[0, name.len() as u8 + 1, name.len() as u8], name.as_bytes()].concat()
}

// Register a package under `name`, once it has compiled.
pub fn add(carbon: &Rc<Carbon>, name: &str, bytes: &[u8]) -> Result<()> {
  let package = crate::wasm::Package::read(bytes)?;
  awaited(|added| carbon.add(name, package, added))
}

// A personal channel running `[probe]` beside `[notes]`, with the timers.
pub fn probed() -> (tempfile::TempDir, Rc<Carbon>, Timers, Identity) {
  let (directory, carbon, timers) = timed();
  add(&carbon, "probe", PROBE).unwrap();
  let identity = alice(&carbon);
  on(&carbon, &identity.personal, &identity, "POST [@carbon]/services", json!({"name": "probe"})).unwrap();
  (directory, carbon, timers, identity)
}

// The same, with the timers it schedules on.
pub fn timed() -> (tempfile::TempDir, Rc<Carbon>, Timers) {
  let directory = tempfile::tempdir().unwrap();
  std::fs::write(directory.path().join("carbondb.toml"), config()).unwrap();
  let timers = Timers::default();
  let carbon = Carbon::open(&directory.path().join("carbondb.toml"), timers.schedule(), tasks()).unwrap();
  register_notes(&carbon);
  (directory, carbon, timers)
}

// The CarbonDB in a directory again, as after a restart, without `[notes]`.
pub fn reopen(directory: &tempfile::TempDir) -> Rc<Carbon> {
  Carbon::open(&directory.path().join("carbondb.toml"), Timers::default().schedule(), tasks()).unwrap()
}

pub fn call_now(carbon: &Rc<Carbon>, space: &Rc<ServiceStore>, actor: &Actor, key: &str, input: Value) -> Result<Settled> {
  now(|reply| carbon.submit(space, actor.clone(), Call::from_key(key).unwrap(), input, reply))
}

pub fn space_now(carbon: &Rc<Carbon>, id: &str) -> Result<Rc<ServiceStore>> {
  now(|opened| carbon.with_store(id, opened))
}

// An op logged for a store and pointed to, as a crash leaves it before it ran.
pub fn left(space: &ServiceStore, op: crate::Op) {
  crate::ops::oplog::append(space.channel.log(), &crate::ops::oplog::LogRow::new(op.clone())).unwrap();
  crate::ops::pointers::write(space.store(), &crate::ops::pointers::Pointer { hash: op.hash, time: op.time, device: op.device }).unwrap();
}

// The ops addressed to a store, from its channel's log, in the shared order.
pub fn entries(space: &ServiceStore) -> Vec<crate::Op> {
  crate::ops::oplog::addressed(space.channel.log(), &space.runs.name).unwrap()
}

pub fn alice(carbon: &Rc<Carbon>) -> Identity {
  found(carbon, "alice", &["notes"])
}

// An identity founded on this device, its personal channel running the
// services named; when this device syncs, once its node has created the
// p2panda spaces.
pub fn found(carbon: &Rc<Carbon>, slug: &str, services: &[&str]) -> Identity {
  let input = json!({"slug": slug, "device": "Pixel9", "services": services});
  let created = awaited(|reply| carbon.submit(carbon.system(), Actor::default(), Call::from_key("POST [@carbon]/identity").unwrap(), input, reply)).unwrap();
  serde_json::from_value(created.value).unwrap()
}

// An identity acting from its first device.
pub fn actor(identity: &Identity) -> Actor {
  Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) }
}

// A call on a channel's handle, as the identity, answered at once.
pub fn on(carbon: &Rc<Carbon>, channel: &str, identity: &Identity, key: &str, input: Value) -> Result<Value> {
  now(|answer| carbon.call(Some(channel), Some(&identity.id), key, input, answer))
}

// A call on a channel's handle, as the identity, once it has answered, as
// one that waits on p2panda does.
pub fn on_awaited(carbon: &Rc<Carbon>, channel: &str, identity: &Identity, key: &str, input: Value) -> Result<Value> {
  awaited(|answer| carbon.call(Some(channel), Some(&identity.id), key, input, answer))
}

// The space a channel names for `[name]`.
pub fn service(carbon: &Rc<Carbon>, channel: &str, name: &str) -> Rc<ServiceStore> {
  carbon.service_space(channel, &Call::from_key(&format!("GET [{name}]/any")).unwrap()).unwrap()
}

// The identity's personal channel's `[notes]`.
pub fn personal_notes(carbon: &Rc<Carbon>, identity: &Identity) -> Rc<ServiceStore> {
  service(carbon, &identity.personal, "notes")
}

// A channel "Room" in the identity's personal channel, running `[notes]`: its id.
pub fn room_channel(carbon: &Rc<Carbon>, identity: &Identity) -> String {
  let created = on(carbon, &identity.personal, identity, "POST [@carbon]/channel", json!({"name": "Room"})).unwrap();
  let room = created["channel"].as_str().unwrap().to_string();
  on(carbon, &room, identity, "POST [@carbon]/services", json!({"name": "notes"})).unwrap();
  room
}

// The `[notes]` of a fresh "Room" channel in the identity's personal channel.
pub fn room(carbon: &Rc<Carbon>, identity: &Identity) -> Rc<ServiceStore> {
  let room = room_channel(carbon, identity);
  service(carbon, &room, "notes")
}



