//! Node bindings for the CarbonDB core. This is the internal seam between the
//! core and the JS service runtime; apps never call it.
//!
//! Everything runs on the JS thread. `apply` opens a write transaction, calls
//! the handler, and chains commit and rollback onto the Promise it returns, so
//! Rust never blocks on JS, and the handler's synchronous `run`, `get` and `all`
//! calls re-enter Rust across its awaits inside the open transaction.
//!
//! Every error crosses as a JSON service error in the message, which the
//! runtime turns back into a ServiceError. A handler's own rejection crosses
//! untouched.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use carbondb_core as core;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;

fn to_napi(error: core::ServiceError) -> Error {
  Error::from_reason(serde_json::to_string(&error).expect("a service error serializes"))
}

fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
  serde_json::from_value(value).map_err(|error| to_napi(core::error::bad_request(error.to_string())))
}

#[napi(object)]
pub struct Stamp {
  pub time: f64,
  pub hash: String,
}

#[napi(object)]
pub struct Outcome {
  pub value: Value,
  pub tables: Vec<String>,
  pub wrote: bool,
}

impl From<core::Outcome> for Outcome {
  fn from(outcome: core::Outcome) -> Self {
    Outcome { value: outcome.value, tables: outcome.tables, wrote: outcome.wrote }
  }
}

// The core's `Side` as JS names it: which of a space's two connections a query
// runs on, 'writer' inside an apply or 'reader' inside a read.
#[napi(string_enum = "lowercase")]
pub enum Side {
  Writer,
  Reader,
}

impl From<Side> for core::Side {
  fn from(side: Side) -> Self {
    match side {
      Side::Writer => core::Side::Writer,
      Side::Reader => core::Side::Reader,
    }
  }
}

// The hex BLAKE3 hash of a value's canonical encoding.
#[napi]
pub fn hash(value: Value) -> String {
  core::hash::hash(&value)
}

#[napi]
pub struct Carbon {
  inner: core::Carbon,
}

#[napi]
impl Carbon {
  #[napi(factory)]
  pub fn open(config: String) -> Result<Carbon> {
    Ok(Carbon { inner: core::Carbon::open(Path::new(&config)).map_err(to_napi)? })
  }

  #[napi]
  pub fn stamp(&self, envelope: Value) -> Stamp {
    let stamp = self.inner.stamp(&envelope);
    Stamp { time: stamp.time as f64, hash: stamp.hash }
  }

  #[napi]
  pub fn identity(&self, slug: String) -> Result<Value> {
    let identity = self.inner.identity(&slug).map_err(to_napi)?;
    serde_json::to_value(identity).map_err(|error| to_napi(core::error::internal(error.to_string())))
  }

  #[napi]
  pub fn open_space(&self, id: String) -> Result<Space> {
    Ok(Space { store: Rc::new(self.inner.open_space(&id).map_err(to_napi)?), busy: Rc::new(Cell::new(false)) })
  }

  #[napi]
  pub fn call_system(&self, call: Value, input: Value) -> Result<Outcome> {
    Ok(self.inner.call_system(&from_json(call)?, input).map_err(to_napi)?.into())
  }

  #[napi]
  pub fn call_space(&self, space: &Space, actor: Value, call: Value, input: Value) -> Result<Outcome> {
    space.idle()?;
    Ok(self.inner.call_space(&space.store, &from_json(actor)?, &from_json(call)?, input).map_err(to_napi)?.into())
  }
}

#[napi]
pub struct Space {
  store: Rc<core::Store>,
  // Whether an async apply or read holds a transaction open on this space.
  busy: Rc<Cell<bool>>,
}

#[napi]
impl Space {
  #[napi]
  pub fn origin(&self) -> Result<Value> {
    let origin = core::space::origin(&self.store, core::Side::Reader).map_err(to_napi)?;
    serde_json::to_value(origin).map_err(|error| to_napi(core::error::internal(error.to_string())))
  }

  // The space a ref here points to, by kind and slug.
  #[napi]
  pub fn find_ref(&self, kind: String, slug: String) -> Result<Option<Value>> {
    let found = core::space::find_ref(&self.store, core::Side::Reader, &kind, &slug).map_err(to_napi)?;
    Ok(found.map(|reference| serde_json::to_value(reference).expect("a ref serializes")))
  }

  #[napi]
  pub fn migrate(&self, service: String, migrations: Vec<String>) -> Result<()> {
    self.idle()?;
    core::space::migrate(&self.store, &service, &migrations).map_err(to_napi)
  }

  // A write statement, on the writer: only an open apply's handler writes.
  #[napi]
  pub fn run(&self, sql: String, params: Option<Vec<Value>>) -> Result<f64> {
    Ok(self.store.run(core::Side::Writer, &sql, &params.unwrap_or_default()).map_err(to_napi)? as f64)
  }

  // The first row, or null: on the writer inside an apply, on the reader inside a read.
  #[napi]
  pub fn get(&self, side: Side, sql: String, params: Option<Vec<Value>>) -> Result<Value> {
    self.store.get(side.into(), &sql, &params.unwrap_or_default()).map_err(to_napi)
  }

  #[napi]
  pub fn all(&self, side: Side, sql: String, params: Option<Vec<Value>>) -> Result<Vec<Value>> {
    self.store.all(side.into(), &sql, &params.unwrap_or_default()).map_err(to_napi)
  }

  // Run an async handler in one write transaction: commit when it resolves, roll
  // back when it rejects or throws, and settle with {value, tables written}.
  // The catch step returns a rejected promise, so the Rust type nests one
  // promise in another; JS flattens them.
  #[napi(ts_return_type = "Promise<{value: unknown, tables: string[]}>")]
  pub fn apply<'env>(&self, handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>) -> Result<PromiseRaw<'env, PromiseRaw<'env, Object<'env>>>> {
    let store = Rc::clone(&self.store);
    self.hold(|| store.begin_write(), handler, move |store, commit| store.end_write(commit))
  }

  // Run an async GET handler on a snapshot of committed state, and settle with
  // {value, tables read}.
  #[napi(ts_return_type = "Promise<{value: unknown, tables: string[]}>")]
  pub fn read<'env>(&self, handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>) -> Result<PromiseRaw<'env, PromiseRaw<'env, Object<'env>>>> {
    let store = Rc::clone(&self.store);
    self.hold(|| store.begin_read(), handler, move |store, _| store.end_read())
  }
}

impl Space {
  // Refuse to start while a transaction is open; ordering calls is the runtime's queue's job.
  fn idle(&self) -> Result<()> {
    if self.busy.get() {
      return Err(to_napi(core::error::conflict("A transaction is already open on this space")));
    }
    Ok(())
  }

  // Hold a transaction open across an async handler, ending it when the
  // handler's Promise settles: `end(store, true)` on success, `end(store, false)`
  // on failure, each returning the tables touched.
  fn hold<'env>(
    &self,
    begin: impl FnOnce() -> core::Result<()>,
    handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>,
    end: impl Fn(&core::Store, bool) -> core::Result<Vec<String>> + Clone + 'env,
  ) -> Result<PromiseRaw<'env, PromiseRaw<'env, Object<'env>>>> {
    self.idle()?;
    begin().map_err(to_napi)?;
    self.busy.set(true);

    let pending = match handler.call(()) {
      Ok(pending) => pending,
      // The handler's error is the one the caller needs; ending the transaction can only add noise.
      Err(error) => {
        let _ = end(&self.store, false);
        self.busy.set(false);
        return Err(error);
      }
    };

    let (store, busy, finish) = (Rc::clone(&self.store), Rc::clone(&self.busy), end.clone());
    let settled = pending.then(move |context| {
      busy.set(false);
      let tables = finish(&store, true).map_err(to_napi)?;
      let mut outcome = Object::new(&context.env)?;
      outcome.set_named_property("value", context.value)?;
      outcome.set_named_property("tables", tables)?;
      Ok(outcome)
    })?;

    // Rejecting with the original value keeps a service error intact: napi-rs
    // turns an Err from a promise callback into a fresh Error, dropping its fields.
    let (store, busy) = (Rc::clone(&self.store), Rc::clone(&self.busy));
    settled.catch(move |context: CallbackContext<Unknown<'env>>| {
      if busy.replace(false) {
        let _ = end(&store, false);
      }
      PromiseRaw::<Object<'env>>::reject(&context.env, context.value)
    })
  }
}
