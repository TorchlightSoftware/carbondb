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
//! The engine and the compiled components are kept for the whole process:
//! compiling is slow, and a component's compiled form depends only on its bytes.

mod host;
mod http;
mod wasi;

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use carbondb_service_bindings::manifest::Manifest;
use carbondb_service_bindings::package::{embed, manifest_of};
use serde_json::{Value, json};
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine, Store};

use crate::error::{Result, ServiceError, bad_request, internal};
use crate::service_call::ServiceCall;
use crate::services::{Service, Setup, timeout};
use crate::{Db, HandlerContext};
use host::{Running, Turn};

// How often an instance that computes without yielding is made to yield.
const TICK: Duration = Duration::from_millis(10);

// The engine every component compiles and runs on, and the epoch it ticks.
static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
  let mut config = Config::new();
  config.wasm_component_model_async(true).epoch_interruption(true);
  let engine = Engine::new(&config).expect("the host's configuration is valid");
  let ticking = engine.weak();
  std::thread::spawn(move || {
    while let Some(engine) = ticking.upgrade() {
      engine.increment_epoch();
      drop(engine);
      std::thread::sleep(TICK);
    }
  });
  engine
});

// What every instance links: the world's host imports, closed WASI, and
// HTTP through the core.
static LINKER: LazyLock<Linker<Turn>> = LazyLock::new(|| {
  let mut linker = Linker::new(&ENGINE);
  host::link(&mut linker).expect("the host imports link");
  wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("WASI 0.2 links");
  wasmtime_wasi::p3::add_to_linker(&mut linker).expect("WASI 0.3 links");
  wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).expect("WASI 0.2 HTTP links");
  wasmtime_wasi_http::p3::add_to_linker(&mut linker).expect("WASI 0.3 HTTP links");
  linker
});

// Every component compiled so far, by version.
static COMPILED: LazyLock<Mutex<HashMap<String, InstancePre<Turn>>>> = LazyLock::new(Mutex::default);

// A built component with its service.toml embedded: a package to register.
pub fn package(component: &[u8], manifest: &str) -> Result<Vec<u8>> {
  Manifest::parse(manifest).map_err(bad_request)?;
  embed(component, manifest).map_err(bad_request)
}

// A package, read and compiled: what it asks of the host, its version, and
// its component, ready to instantiate.
pub struct Package {
  pub manifest: Manifest,
  pub version: String,
  ready: InstancePre<Turn>,
}

impl Package {
  // A package from its bytes: refused unless it is a component of the
  // `carbondb:service` world carrying a service.toml.
  pub fn read(bytes: &[u8]) -> Result<Package> {
    let text = manifest_of(bytes).map_err(bad_request)?.ok_or_else(|| bad_request("This component carries no service.toml; package it first"))?;
    let manifest = Manifest::parse(&text).map_err(bad_request)?;
    let version = carbondb_service_bindings::hash::of_bytes(bytes);
    Ok(Package { manifest, ready: compiled(&version, bytes)?, version })
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
      setup,
      entry,
    }
  }

  // Set the service's schema up: `setup`, run to completion.
  fn setup(&self, db: Rc<dyn Db>) -> Result<()> {
    let running = Running::register(host::Context::Setup(db));
    let mut store = self.store(running.id(), None);
    finish(async {
      let instance = self.ready.instantiate_async(&mut store).await?;
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
    let answered = async {
      let instance = self.ready.instantiate_async(&mut *store).await?;
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

// A component compiled once per process, by version.
fn compiled(version: &str, bytes: &[u8]) -> Result<InstancePre<Turn>> {
  if let Some(ready) = COMPILED.lock().expect("the compile cache is never poisoned").get(version) {
    return Ok(ready.clone());
  }
  let component = Component::new(&ENGINE, bytes).map_err(|error| bad_request(format!("Not a component this host can run: {error}")))?;
  let ready = LINKER.instantiate_pre(&component).map_err(|error| bad_request(format!("A component of another world than carbondb:service: {error}")))?;
  COMPILED.lock().expect("the compile cache is never poisoned").insert(version.to_string(), ready.clone());
  Ok(ready)
}

// A call as `handle` receives it, with the name the service gives its binding.
fn request_of(call: &ServiceCall, binding: Option<&str>) -> String {
  json!({"method": call.method(), "resource": call.resource(), "input": call.input(), "actor": call.actor(), "op": call.op(), "binding": binding}).to_string()
}

// A trap inside an instance, as the call's failure.
fn trapped(error: wasmtime::Error) -> ServiceError {
  internal(format!("The service failed: {error:?}"))
}

// Run a future to completion here and now. Only `setup` runs this way: it
// reaches nothing but its binding, so it never waits on anything else, and
// an epoch yield is ready to poll again at once.
fn finish<F: Future>(future: F) -> F::Output {
  let mut future = pin!(future);
  let mut context = Context::from_waker(Waker::noop());
  loop {
    if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
      return output;
    }
  }
}
