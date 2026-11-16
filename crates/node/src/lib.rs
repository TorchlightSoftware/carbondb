//! Node bindings for the CarbonDB core. This is the internal seam between the
//! core and the JS runtime; apps never call it.
//!
//! The core runs on a thread of its own (`carbondb::Runtime`), and every
//! service runs inside it as a WebAssembly component, so no service code runs
//! on Node's thread. The binding only passes work and answers across:
//!
//! - JS makes a call with `call`, by its service key and as an identity, and
//!   the core answers it once the op applies and the watches it touched have
//!   run again.
//! - JS watches a GET or a space's RunState with `watch` and `watchRunState`,
//!   and the core answers each run, until `unwatch`. The core keeps every
//!   watch; JS only hands answers to the app.
//! - JS registers a service with `register`: the bytes of its package, a
//!   component carrying its service.toml.
//!
//! Every later answer crosses back to Node's thread through one courier, a
//! thread-safe function JS gave at open, in the order the core gave them, with
//! the id JS gave the call or watch it answers. JS refs the courier while it
//! is owed an answer, so Node keeps running for it. What JS reads at once,
//! such as a ref or a RunState, waits for the core's thread to answer.
//!
//! Every error crosses as a JSON service error: in an Error's message, or a
//! callback's first argument.

use std::path::Path;
use std::sync::{Arc, Mutex};

use carbondb as core;
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use serde_json::{Value, json};

fn to_napi(error: core::ServiceError) -> Error {
  Error::from_reason(error.to_text())
}

fn to_json(value: impl serde::Serialize) -> core::Result<Value> {
  serde_json::to_value(value).map_err(|error| core::error::internal(error.to_string()))
}

// The courier every later answer crosses back through, in order: a JS
// function called with `(id, error, value)`, the id naming the call or watch
// it answers. JS refs it while it is owed an answer.
type Courier = ThreadsafeFunction<(u32, Value, Value), (), FnArgs<(u32, Value, Value)>, Status, false>;

// Hand the answer to `id` to JS, on Node's thread.
fn deliver(courier: &Mutex<Courier>, id: u32, result: core::Result<Value>) {
  let (error, value) = match result {
    Ok(value) => (Value::Null, value),
    Err(error) => (json!(error), Value::Null),
  };
  courier.lock().expect("never poisoned").call((id, error, value), ThreadsafeFunctionCallMode::NonBlocking);
}

#[napi]
pub struct Carbon {
  runtime: core::Runtime,
  courier: Arc<Mutex<Courier>>,
}

#[napi]
impl Carbon {
  // Open CarbonDB from its config, on a thread of its own, answering later
  // through `courier`, which keeps Node running only while JS says so.
  #[napi(factory)]
  pub fn open(env: Env, config: String, courier: Function<'_, FnArgs<(u32, Value, Value)>, ()>) -> Result<Carbon> {
    let mut courier = courier.build_threadsafe_function::<(u32, Value, Value)>().callee_handled::<false>().build_callback(|context| Ok(FnArgs::from(context.value)))?;
    #[allow(deprecated)]
    courier.unref(&env)?;
    Ok(Carbon { runtime: core::Runtime::start(Path::new(&config)).map_err(to_napi)?, courier: Arc::new(Mutex::new(courier)) })
  }

  // Whether Node keeps running for answers still owed: JS says so as it owes them.
  #[napi]
  #[allow(deprecated)]
  pub fn keep(&self, env: Env, keep: bool) -> Result<()> {
    let mut courier = self.courier.lock().expect("never poisoned");
    match keep {
      true => courier.refer(&env),
      false => courier.unref(&env),
    }
  }

  // Register a service under a name: the bytes of its package.
  #[napi]
  pub fn register(&self, name: String, package: Buffer) -> Result<()> {
    let bytes = package.to_vec();
    self.runtime.ask(move |carbon| carbon.register(core::wasm::Package::read(&bytes)?.service(&name))).map_err(to_napi)
  }

  // Make a call by its service key on a channel, or on the system handle when
  // none is named, as the identity named or as no one; its answer goes to `id`.
  #[napi]
  pub fn call(&self, channel: Option<String>, identity: Option<String>, key: String, input: Value, id: u32) {
    let courier = Arc::clone(&self.courier);
    self.runtime.with(move |carbon| carbon.call(channel.as_deref(), identity.as_deref(), &key, input, Box::new(move |answer| deliver(&courier, id, answer))));
  }

  // Run a GET on a channel now and after every commit that writes what it
  // read, as the identity named; each answer goes to `id`. Returns the id
  // that stops it.
  #[napi]
  pub fn watch(&self, channel: String, identity: Option<String>, key: String, id: u32) -> Result<u32> {
    let courier = Arc::clone(&self.courier);
    self.runtime.ask(move |carbon| carbon.watch(&channel, identity.as_deref(), &key, std::rc::Rc::new(move |answer| deliver(&courier, id, answer)))).map_err(to_napi)
  }

  // The ref in a channel to `kind:slug`, found at once: the channel it points to, its kind, name and slug.
  #[napi]
  pub fn find_ref(&self, channel: String, kind: String, slug: String) -> Result<Value> {
    self.runtime.ask(move |carbon| carbon.find_ref(&channel, &kind, &slug)).map_err(to_napi)
  }

  // The id of the service a channel names `[name]`; the channel's own for `[channel]`.
  #[napi]
  pub fn service(&self, channel: String, name: String) -> Result<String> {
    self.runtime.ask(move |carbon| carbon.named(&channel, &name)).map_err(to_napi)
  }

  // Stop a watch or a RunState listener.
  #[napi]
  pub fn unwatch(&self, id: u32) {
    self.runtime.with(move |carbon| carbon.unwatch(id))
  }

  // A space's RunState: `{state: 'initializing', services}`, `{state: 'idle'}`,
  // `{state: 'applying', op}` or `{state: 'stopped', error}`.
  #[napi]
  pub fn run_state(&self, space: String) -> Result<Value> {
    self.runtime.ask(move |carbon| to_json(carbon.run_state(&space))).map_err(to_napi)
  }

  // Hear every change of a space's RunState, each going to `id`. Returns the id that stops it.
  #[napi]
  pub fn watch_run_state(&self, space: String, id: u32) -> Result<u32> {
    let courier = Arc::clone(&self.courier);
    self.runtime.ask(move |carbon| Ok(carbon.watch_run_state(&space, std::rc::Rc::new(move |state: &core::RunState| deliver(&courier, id, Ok(json!(state)))))))
      .map_err(to_napi)
  }

  // Resume a space a system failure stopped, once the failure is fixed;
  // answered at `id` once the watches on it have run again.
  #[napi]
  pub fn resume(&self, space: String, id: u32) {
    let courier = Arc::clone(&self.courier);
    self.runtime.with(move |carbon| carbon.resume_space(&space, Box::new(move |answer| deliver(&courier, id, answer))));
  }

  #[napi]
  pub fn identity(&self, slug: String) -> Result<Value> {
    self.runtime.ask(move |carbon| to_json(carbon.identity(&slug)?)).map_err(to_napi)
  }
}

// Embed a service.toml in a built component, making it a package to register.
#[napi]
pub fn package_component(component: Buffer, manifest: String) -> Result<Buffer> {
  core::wasm::package(&component, &manifest).map(Buffer::from).map_err(to_napi)
}
