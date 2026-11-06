//! Node bindings for the CarbonDB core. This is the internal seam between the
//! core and the JS runtime; apps never call it.
//!
//! The core routes every call; the binding only translates. Everything runs on
//! the JS thread, in callback style:
//!
//! - JS submits a call with `call`, and the core answers through the callback
//!   it was given, `(error, outcome)`, now or once the op applies.
//! - JS registers an npm service with `register`: its `init`, which the core
//!   calls with the service's `db` binding to set up its schema, and its entry
//!   point, which the core calls with a `HandlerContext` for each call to it.
//!   The entry point settles that context once, when its handler's Promise
//!   settles. Until then the handler's synchronous `run`, `get` and `all`
//!   re-enter the core inside the open transaction or snapshot.
//!
//! Every error crosses as a JSON service error: from the core, in an Error's
//! message or a callback's first argument; from JS, as the data `settle` or
//! `init` hands back.

use std::rc::Rc;

use carbondb_core as core;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{Value, json};

fn to_napi(error: core::ServiceError) -> Error {
  Error::from_reason(serde_json::to_string(&error).expect("a service error serializes"))
}

fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
  serde_json::from_value(value).map_err(|error| to_napi(core::error::bad_request(error.to_string())))
}

fn to_json(value: impl serde::Serialize) -> Result<Value> {
  serde_json::to_value(value).map_err(|error| to_napi(core::error::internal(error.to_string())))
}

// A service error JS handed back as data, or an internal error if it is not one.
fn service_error(data: Value) -> core::ServiceError {
  serde_json::from_value(data.clone()).unwrap_or_else(|_| core::error::internal(format!("Malformed service error: {data}")))
}

// A JS callback, `(error, value)`, as the core's answer to a call: called once,
// later, on the JS thread whose env it was given on.
type Answer = Box<dyn FnOnce(core::Result<Value>)>;

fn answer(env: &Env, callback: Function<'_, FnArgs<(Value, Value)>, ()>) -> Result<Answer> {
  let callback = callback.create_ref()?;
  let raw = env.raw();
  Ok(Box::new(move |result| {
    let env = Env::from_raw(raw);
    let args = match result {
      Ok(value) => (Value::Null, value),
      Err(error) => (json!(error), Value::Null),
    };
    // The runtime's callbacks only settle a Promise; they never throw.
    if let Ok(callback) = callback.borrow_back(&env) {
      let _ = callback.call(args.into());
    }
  }))
}

// The hex BLAKE3 hash of a value's canonical encoding.
#[napi]
pub fn hash(value: Value) -> String {
  core::hash::hash(&value)
}

#[napi]
pub struct Carbon {
  inner: Rc<core::Carbon>,
}

#[napi]
impl Carbon {
  #[napi(factory)]
  pub fn open(config: String) -> Result<Carbon> {
    Ok(Carbon { inner: core::Carbon::open(std::path::Path::new(&config)).map_err(to_napi)? })
  }

  // Register an npm service: its version, its `sql` binding, its `init`, which
  // answers null or a service error's data, and its entry point, which the core
  // calls with a HandlerContext for each call to the service.
  #[napi]
  pub fn register(
    &self,
    env: Env,
    name: String,
    version: String,
    binding: Option<String>,
    init: Function<'_, Db, Value>,
    entry: Function<'_, HandlerContext, ()>,
  ) -> Result<()> {
    let raw = env.raw();
    let init = init.create_ref()?;
    let set_up: core::Init = Rc::new(move |db: Rc<dyn core::Db>| {
      let env = Env::from_raw(raw);
      let returned = init.borrow_back(&env).and_then(|init| init.call(Db { inner: db }));
      match returned {
        Ok(Value::Null) => Ok(()),
        Ok(error) => Err(service_error(error)),
        Err(error) => Err(core::error::internal(error.reason.clone())),
      }
    });
    let entry = entry.create_ref()?;
    let run: core::Entry = Rc::new(move |context: Rc<dyn core::HandlerContext>| {
      let env = Env::from_raw(raw);
      let called = entry.borrow_back(&env).and_then(|entry| entry.call(HandlerContext { inner: Rc::clone(&context) }));
      if let Err(error) = called {
        let _ = context.settle(Err(core::error::internal(error.reason.clone())));
      }
    });
    self.inner.register(core::Service::replicated(&name, &version, binding.as_deref(), set_up, run)).map_err(to_napi)
  }

  // Submit a call to a space, opening it first, or to the system handle when no
  // space is named. `reply` is called with (error, {value, tables}).
  #[napi]
  pub fn call(&self, env: Env, space: Option<String>, actor: Value, call: Value, input: Value, reply: Function<'_, FnArgs<(Value, Value)>, ()>) -> Result<()> {
    let reply = answer(&env, reply)?;
    let (actor, call): (core::Actor, core::Call) = (from_json(actor)?, from_json(call)?);
    let carbon = Rc::clone(&self.inner);
    let submit = move |space: core::Result<Rc<core::Space>>| match space {
      Ok(space) => carbon.submit(&space, actor, call, input, Box::new(move |settled| reply(settled.map(|settled| json!(settled))))),
      Err(error) => reply(Err(error)),
    };
    match space {
      Some(id) => Rc::clone(&self.inner).with_space(&id, Box::new(submit)),
      None => submit(Ok(Rc::clone(self.inner.system()))),
    }
    Ok(())
  }

  // Hear every commit to a space, with the tables it wrote.
  #[napi]
  pub fn watch(&self, env: Env, space: String, listener: Function<'_, Vec<String>, ()>) -> Result<()> {
    let listener = listener.create_ref()?;
    let raw = env.raw();
    self.inner.listen(&space, Rc::new(move |tables: &[String]| {
      let env = Env::from_raw(raw);
      if let Ok(listener) = listener.borrow_back(&env) {
        let _ = listener.call(tables.to_vec());
      }
    }));
    Ok(())
  }

  // A space's RunState: `{state: 'initializing', services}`, `{state: 'idle'}`,
  // `{state: 'applying', op}` or `{state: 'stopped', error}`.
  #[napi]
  pub fn run_state(&self, space: String) -> Result<Value> {
    to_json(self.inner.run_state(&space))
  }

  // Hear every change of a space's RunState.
  #[napi]
  pub fn watch_run_state(&self, env: Env, space: String, listener: Function<'_, Value, ()>) -> Result<()> {
    let listener = listener.create_ref()?;
    let raw = env.raw();
    self.inner.listen_run_state(&space, Rc::new(move |state: &core::RunState| {
      let env = Env::from_raw(raw);
      if let Ok(listener) = listener.borrow_back(&env) {
        let _ = listener.call(json!(state));
      }
    }));
    Ok(())
  }

  // Resume a space a system failure stopped, once the failure is fixed.
  #[napi]
  pub fn resume(&self, space: String) {
    let carbon = Rc::clone(&self.inner);
    Rc::clone(&self.inner).with_space(&space, Box::new(move |opened| {
      if let Ok(space) = opened {
        carbon.resume(&space)
      }
    }));
  }

  #[napi]
  pub fn identity(&self, slug: String) -> Result<Value> {
    to_json(self.inner.identity(&slug).map_err(to_napi)?)
  }

  // The ids of every space on this device.
  #[napi]
  pub fn spaces(&self) -> Result<Vec<String>> {
    self.inner.space_ids().map_err(to_napi)
  }
}

// A service's `db` binding as its `init` receives it, open while `init` runs.
#[napi]
pub struct Db {
  inner: Rc<dyn core::Db>,
}

#[napi]
impl Db {
  #[napi]
  pub fn run(&self, sql: String, params: Option<Vec<Value>>) -> Result<f64> {
    Ok(self.inner.run(&sql, &params.unwrap_or_default()).map_err(to_napi)? as f64)
  }

  #[napi]
  pub fn get(&self, sql: String, params: Option<Vec<Value>>) -> Result<Value> {
    self.inner.get(&sql, &params.unwrap_or_default()).map_err(to_napi)
  }

  #[napi]
  pub fn all(&self, sql: String, params: Option<Vec<Value>>) -> Result<Vec<Value>> {
    self.inner.all(&sql, &params.unwrap_or_default()).map_err(to_napi)
  }
}

// One call as an npm service's entry point receives it: its handler context.
#[napi]
pub struct HandlerContext {
  inner: Rc<dyn core::HandlerContext>,
}

#[napi]
impl HandlerContext {
  // The service call as data: method, resource, input, and the op when it writes.
  #[napi]
  pub fn request(&self) -> Value {
    json!({"method": self.inner.method(), "resource": self.inner.resource(), "input": self.inner.input(), "op": self.inner.op()})
  }

  // A write statement, inside the op's transaction.
  #[napi]
  pub fn run(&self, sql: String, params: Option<Vec<Value>>) -> Result<f64> {
    Ok(self.inner.run(&sql, &params.unwrap_or_default()).map_err(to_napi)? as f64)
  }

  // The first row, or null.
  #[napi]
  pub fn get(&self, sql: String, params: Option<Vec<Value>>) -> Result<Value> {
    self.inner.get(&sql, &params.unwrap_or_default()).map_err(to_napi)
  }

  #[napi]
  pub fn all(&self, sql: String, params: Option<Vec<Value>>) -> Result<Vec<Value>> {
    self.inner.all(&sql, &params.unwrap_or_default()).map_err(to_napi)
  }

  // An id drawn from the op.
  #[napi]
  pub fn id(&self) -> Result<String> {
    self.op().map(|op| op.id())
  }

  // A number in [0, 1) drawn from the op.
  #[napi]
  pub fn random(&self) -> Result<f64> {
    self.op().map(|op| op.random())
  }

  // Make a call by its service key from inside this one; `reply` is called with (error, value).
  #[napi]
  pub fn call(&self, env: Env, key: String, input: Value, reply: Function<'_, FnArgs<(Value, Value)>, ()>) -> Result<()> {
    let reply = answer(&env, reply)?;
    Rc::clone(&self.inner).call(&key, input, reply);
    Ok(())
  }

  // Settle the call, once: with a service error's data, or with a value.
  #[napi]
  pub fn settle(&self, error: Option<Value>, value: Option<Value>) -> Result<()> {
    let result = match error {
      Some(error) => Err(service_error(error)),
      None => Ok(value.unwrap_or(Value::Null)),
    };
    self.inner.settle(result).map_err(to_napi)
  }
}

impl HandlerContext {
  fn op(&self) -> Result<&core::Op> {
    self.inner.op().ok_or_else(|| to_napi(core::error::bad_request("A GET is not an op, so it draws nothing")))
  }
}
