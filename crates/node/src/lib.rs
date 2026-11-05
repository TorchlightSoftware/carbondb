//! Node bindings for the CarbonDB core. This is the internal seam between the
//! core and the JS runtime; apps never call it.
//!
//! The core routes every call; the binding only translates. Everything runs on
//! the JS thread, in callback style:
//!
//! - JS submits a call with `call`, and the core answers through the callback
//!   it was given, `(error, outcome)`, now or once the op applies.
//! - JS registers an npm service with `register`. The core calls its entry
//!   point with an `Invocation` for each call to it, and the entry point
//!   settles that invocation once, when its handler's Promise settles. Until
//!   then the handler's synchronous `run`, `get` and `all` re-enter the core
//!   inside the open transaction or snapshot.
//!
//! Every error crosses as a JSON service error: from the core, in an Error's
//! message or a callback's first argument; from a handler, as the data
//! `settle` is given.

use std::rc::Rc;

use carbondb_core as core;
use carbondb_core::Db;
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

  // Register an npm service: its migrations, and its entry point, which the core
  // calls with an Invocation for each call to the service.
  #[napi]
  pub fn register(&self, env: Env, name: String, migrations: Vec<String>, entry: Function<'_, Invocation, ()>) -> Result<()> {
    let entry = entry.create_ref()?;
    let raw = env.raw();
    let run: core::Entry = Rc::new(move |invocation: Rc<core::Invocation>| {
      let env = Env::from_raw(raw);
      let called = entry.borrow_back(&env).and_then(|entry| entry.call(Invocation { inner: Rc::clone(&invocation) }));
      if let Err(error) = called {
        let _ = invocation.settle(Err(core::error::internal(error.reason.clone())));
      }
    });
    self.inner.services().register(core::Service { name, local: false, migrations, entry: run }).map_err(to_napi)
  }

  // Submit a call to a space, or to the system handle when no space is named.
  // `reply` is called with (error, {value, tables}).
  #[napi]
  pub fn call(&self, env: Env, space: Option<String>, actor: Value, call: Value, input: Value, reply: Function<'_, FnArgs<(Value, Value)>, ()>) -> Result<()> {
    let space = self.space(space)?;
    let reply = answer(&env, reply)?;
    let (actor, call) = (from_json(actor)?, from_json(call)?);
    self.inner.submit(&space, actor, call, input, Box::new(move |settled| reply(settled.map(|settled| json!(settled)))));
    Ok(())
  }

  // Hear every commit to a space, with the tables it wrote.
  #[napi]
  pub fn watch(&self, env: Env, space: String, listener: Function<'_, Vec<String>, ()>) -> Result<()> {
    let listener = listener.create_ref()?;
    let raw = env.raw();
    self.space(Some(space))?.listen(Rc::new(move |tables: &[String]| {
      let env = Env::from_raw(raw);
      if let Ok(listener) = listener.borrow_back(&env) {
        let _ = listener.call(tables.to_vec());
      }
    }));
    Ok(())
  }

  #[napi]
  pub fn identity(&self, slug: String) -> Result<Value> {
    to_json(self.inner.identity(&slug).map_err(to_napi)?)
  }

  // The space a ref in a space points to, by kind and slug.
  #[napi]
  pub fn find_ref(&self, space: String, kind: String, slug: String) -> Result<Option<Value>> {
    let space = self.space(Some(space))?;
    let reader = space.store().reader().map_err(to_napi)?;
    let found = core::builtins::space::find_ref(&reader, &kind, &slug).map_err(to_napi)?;
    found.map(to_json).transpose()
  }
}

impl Carbon {
  fn space(&self, id: Option<String>) -> Result<Rc<core::Space>> {
    match id {
      Some(id) => self.inner.space(&id).map_err(to_napi),
      None => Ok(Rc::clone(self.inner.system())),
    }
  }
}

// One call as an npm service's entry point receives it.
#[napi]
pub struct Invocation {
  inner: Rc<core::Invocation>,
}

#[napi]
impl Invocation {
  // The call as data: key, method, resource, input, and the op when it writes.
  #[napi]
  pub fn request(&self) -> Value {
    self.inner.request()
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

  // The next value drawn from the op, the source of its ids and randomness.
  #[napi]
  pub fn draw(&self) -> Result<String> {
    self.inner.draw().map_err(to_napi)
  }

  // Make a call from inside this one; `reply` is called with (error, value).
  #[napi]
  pub fn call(&self, env: Env, call: Value, input: Value, reply: Function<'_, FnArgs<(Value, Value)>, ()>) -> Result<()> {
    let reply = answer(&env, reply)?;
    self.inner.call(from_json(call)?, input, reply);
    Ok(())
  }

  // Settle the call, once: with a service error's data, or with a value.
  #[napi]
  pub fn settle(&self, error: Option<Value>, value: Option<Value>) -> Result<()> {
    let result = match error {
      Some(error) => Err(serde_json::from_value(error.clone()).unwrap_or_else(|_| core::error::internal(format!("Malformed service error: {error}")))),
      None => Ok(value.unwrap_or(Value::Null)),
    };
    self.inner.settle(result).map_err(to_napi)
  }
}
