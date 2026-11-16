//! The WASM host: every service a channel names runs as a WebAssembly
//! component of the `carbondb:service` world (ServiceBindings' `wit/`), built
//! from any language. A package is one component carrying its `service.toml`
//! (`carbondb_service_bindings::package`); its version is the hash of its
//! bytes, so the hash covers its manifest too.
//!
//! Each call runs in an instance of its own, as a task on the core's thread:
//! the core hands the component the call as `handle`'s request, and settles
//! the call with what `handle` answers. A service keeps nothing in its
//! instance between calls; its state is its `sql` binding. Its `setup` runs
//! to completion on the spot, as the core sets a space up.
//!
//! What a component reaches of the outside world is what the host links: the
//! world's `host` imports, which reach the core (`host`); WASI, closed
//! (`wasi`); and `wasi:http`, whose every request is the core's `fetch`
//! effect (`http`). An instance that computes without yielding is made to yield
//! every epoch tick, so a call's deadline can pass and abandon it.
//!
//! A component compiles off the core's thread, once per process, and its
//! compiled code is kept on disk (`compile`).

mod compile;
mod host;
mod http;
mod wasi;

pub use compile::{Compiled, keep_in};

use std::rc::Rc;

use carbondb_service_bindings::manifest::Manifest;
use carbondb_service_bindings::package::{embed, manifest_of};
use serde_json::{Value, json};
use wasmtime::Store;
use wasmtime::component::InstancePre;

use crate::error::{Result, ServiceError, bad_request, internal};
use crate::calls::service_call::ServiceCall;
use crate::services::{Service, Setup, startup_timeout, timeout};
use crate::{Db, HandlerContext};
use compile::{ENGINE, block_on, compiled};
use host::{Running, Turn};

// A built component with its service.toml embedded: a package to register.
pub fn package(component: &[u8], manifest: &str) -> Result<Vec<u8>> {
  Manifest::parse(manifest).map_err(bad_request)?;
  embed(component, manifest).map_err(bad_request)
}

// A package, read: what it asks of the host, its version, and its
// component, compiling on the pool from the moment it is read.
pub struct Package {
  pub manifest: Manifest,
  pub version: String,
  compiled: Compiled,
}

impl Package {
  // A package from its bytes, refused unless it carries a service.toml; it
  // starts compiling, and a component of another world than
  // `carbondb:service` fails to.
  pub fn read(bytes: &[u8]) -> Result<Package> {
    let text = manifest_of(bytes).map_err(bad_request)?.ok_or_else(|| bad_request("This component carries no service.toml; package it first"))?;
    let manifest = Manifest::parse(&text).map_err(bad_request)?;
    let version = carbondb_service_bindings::hash::of_bytes(bytes);
    Ok(Package { manifest, compiled: compiled(&version, bytes), version })
  }

  // Its component, compiling or compiled.
  pub fn compiled(&self) -> Compiled {
    self.compiled.clone()
  }

  // Its compiled component: a service is set up and called only once it is.
  fn ready(&self) -> Result<InstancePre<Turn>> {
    self.compiled.now().unwrap_or_else(|| Err(internal("A service ran before its component compiled")))
  }

  // The service it runs, registered under `name`.
  pub fn service(self, name: &str) -> Service {
    let package = Rc::new(self);
    let (setting_up, running) = (Rc::clone(&package), Rc::clone(&package));
    let setup: Setup = Rc::new(move |db: Rc<dyn Db>| setting_up.setup(db));
    let entry = Rc::new(move |call: Rc<ServiceCall>| running.start(call));
    Service {
      name: name.into(),
      local: false,
      version: package.version.clone(),
      binding: package.manifest.binding().map(str::to_string),
      timeout: timeout(package.manifest.timeout),
      always: package.manifest.replicated_always(),
      health: package.manifest.health.clone(),
      startup: startup_timeout(package.manifest.startup_timeout),
      setup,
      entry,
    }
  }

  // Set the service's schema up: `setup`, run to completion. It reaches
  // nothing but its binding, so it never waits on anything else.
  fn setup(&self, db: Rc<dyn Db>) -> Result<()> {
    let ready = self.ready()?;
    let running = Running::register(host::Context::Setup(db));
    let mut store = self.store(running.id(), None);
    block_on(async {
      let instance = ready.instantiate_async(&mut store).await?;
      let setup = instance.get_typed_func::<(), (std::result::Result<(), String>,)>(&mut store, "setup")?;
      setup.call_async(&mut store, ()).await
    })
    .map_err(trapped)?
    .0
    .map_err(|text| ServiceError::from_text(&text))
  }

  // Run a call as a task: `handle`, in an instance of its own, settling the
  // call with its answer. When the call expires first, the task is dropped.
  fn start(self: &Rc<Self>, call: Rc<ServiceCall>) {
    let running = Running::register(host::Context::Call(Rc::clone(&call)));
    let request = request_of(call.as_ref(), self.manifest.binding());
    let mut store = self.store(running.id(), call.op().map(|op| (op.hash.as_str(), op.time)));
    let (package, settling) = (Rc::clone(self), Rc::clone(&call));
    let task = async move {
      let answer = package.handle(&mut store, request).await;
      drop(running);
      let _ = settling.settle(answer);
    };
    let cancel = call.carbon().spawn(Box::pin(task));
    call.abandon_with(cancel);
  }

  async fn handle(&self, store: &mut Store<Turn>, request: String) -> Result<Value> {
    let ready = self.ready()?;
    let answered = async {
      let instance = ready.instantiate_async(&mut *store).await?;
      let handle = instance.get_typed_func::<(String,), (std::result::Result<String, String>,)>(&mut *store, "handle")?;
      store.run_concurrent(async |accessor| handle.call_concurrent(accessor, (request,)).await).await?
    };
    match answered.await.map_err(trapped)?.0 {
      Ok(text) => serde_json::from_str(&text).map_err(|error| internal(format!("The service answered with malformed JSON: {error}"))),
      Err(text) => Err(ServiceError::from_text(&text)),
    }
  }

  // A store for one instance, reaching what is registered as `id`; `op`, the
  // hash and time of the op a write runs in, seeds its WASI.
  fn store(&self, id: u64, op: Option<(&str, u64)>) -> Store<Turn> {
    let mut store = Store::new(&ENGINE, Turn::new(id, op));
    store.set_epoch_deadline(1);
    store.epoch_deadline_async_yield_and_update(1);
    store
  }
}

// How many calls and inits instances run for on this thread, now.
#[cfg(test)]
pub(crate) fn running() -> usize {
  host::count()
}

// A call as `handle` receives it, with the name the service gives its binding.
fn request_of(call: &ServiceCall, binding: Option<&str>) -> String {
  json!({"method": call.method(), "resource": call.resource(), "input": call.input(), "actor": call.actor(), "op": call.op(), "binding": binding}).to_string()
}

// A trap inside an instance, as the call's failure.
fn trapped(error: wasmtime::Error) -> ServiceError {
  internal(format!("The service failed: {error:?}"))
}
