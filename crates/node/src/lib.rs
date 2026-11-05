//! Node bindings for the CarbonDB core. This is the internal seam between the
//! core and the JS service runtime; apps never call it.
//!
//! Everything runs on the JS thread. Each side of a space, writer and reader,
//! has a lane: the core's queue of jobs waiting for that side, and whether a
//! transaction is open on it. A job is a JS function the runtime enqueues; its
//! turn comes when the job before it is done. A job runs `apply` or `read`,
//! which open a transaction, call the handler, and chain commit and rollback
//! onto the Promise it returns, so Rust never blocks on JS, and the handler's
//! synchronous `run`, `get` and `all` calls re-enter Rust across its awaits
//! inside the open transaction. A job is done when it returns without leaving a
//! transaction open, or when the transaction it opened ends.
//!
//! Every error crosses as a JSON service error in the message, which the
//! runtime turns back into a ServiceError. A handler's own rejection crosses
//! untouched.

use std::cell::{Cell, RefCell};
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

fn to_json(value: impl serde::Serialize) -> Result<Value> {
  serde_json::to_value(value).map_err(|error| to_napi(core::error::internal(error.to_string())))
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
// or a job runs on, 'writer' or 'reader'.
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
    to_json(self.inner.identity(&slug).map_err(to_napi)?)
  }

  #[napi]
  pub fn open_space(&self, id: String) -> Result<Space> {
    Ok(Space { store: Rc::new(self.inner.open_space(&id).map_err(to_napi)?), writer: Rc::default(), reader: Rc::default() })
  }

  // A call on the system handle.
  #[napi]
  pub fn call_system(&self, call: Value, input: Value) -> Result<Outcome> {
    Ok(self.inner.call(core::Context::System, &core::Actor::default(), &from_json(call)?, input).map_err(to_napi)?.into())
  }

  // A call the core answers in a space, run as a job on the side its method needs.
  #[napi]
  pub fn call_space(&self, space: &Space, actor: Value, call: Value, input: Value) -> Result<Outcome> {
    let call: core::Call = from_json(call)?;
    space.lane(if call.method() == "GET" { core::Side::Reader } else { core::Side::Writer }).idle()?;
    Ok(self.inner.call(core::Context::Space(&space.store), &from_json(actor)?, &call, input).map_err(to_napi)?.into())
  }
}

// A job waiting for its side of a space: a JS function, called when its turn comes.
type Job = FunctionRef<(), ()>;

// One side of a space: the jobs waiting for it, and whether a transaction is open on it.
#[derive(Default)]
struct Lane {
  queue: RefCell<core::Queue<Job>>,
  open: Cell<bool>,
}

impl Lane {
  fn enqueue(&self, env: &Env, job: Job) -> Result<()> {
    let start = self.queue.borrow_mut().push(job);
    match start {
      Some(job) => self.run_from(env, job),
      None => Ok(()),
    }
  }

  // The open transaction ended: start the next job.
  fn next(&self, env: &Env) -> Result<()> {
    let start = self.queue.borrow_mut().finish();
    match start {
      Some(job) => self.run_from(env, job),
      None => Ok(()),
    }
  }

  // Start jobs in turn until one leaves a transaction open, or none are waiting.
  // A job never throws: the runtime settles whatever its work throws.
  fn run_from(&self, env: &Env, first: Job) -> Result<()> {
    let mut job = first;
    loop {
      job.borrow_back(env)?.call(())?;
      if self.open.get() {
        return Ok(());
      }
      let next = self.queue.borrow_mut().finish();
      match next {
        Some(waiting) => job = waiting,
        None => return Ok(()),
      }
    }
  }

  // Refuse to start while a transaction is open; ordering jobs is the lane's job.
  fn idle(&self) -> Result<()> {
    if self.open.get() {
      return Err(to_napi(core::error::conflict("A transaction is already open on this side of the space")));
    }
    Ok(())
  }
}

#[napi]
pub struct Space {
  store: Rc<core::Store>,
  writer: Rc<Lane>,
  reader: Rc<Lane>,
}

#[napi]
impl Space {
  #[napi]
  pub fn origin(&self) -> Result<Value> {
    to_json(core::space::origin(&self.store, core::Side::Reader).map_err(to_napi)?)
  }

  // The space a ref here points to, by kind and slug.
  #[napi]
  pub fn find_ref(&self, kind: String, slug: String) -> Result<Option<Value>> {
    let found = core::space::find_ref(&self.store, core::Side::Reader, &kind, &slug).map_err(to_napi)?;
    found.map(to_json).transpose()
  }

  #[napi]
  pub fn migrate(&self, service: String, migrations: Vec<String>) -> Result<()> {
    self.writer.idle()?;
    core::space::migrate(&self.store, &service, &migrations).map_err(to_napi)
  }

  // Queue a job for one side of the space; it is called when its turn comes.
  #[napi]
  pub fn enqueue(&self, env: Env, side: Side, job: Function<'_, (), ()>) -> Result<()> {
    self.lane(side.into()).enqueue(&env, job.create_ref()?)
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
    self.hold(core::Side::Writer, || store.begin_write(), handler, move |store, commit| store.end_write(commit))
  }

  // Run an async GET handler on a snapshot of committed state, and settle with
  // {value, tables read}.
  #[napi(ts_return_type = "Promise<{value: unknown, tables: string[]}>")]
  pub fn read<'env>(&self, handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>) -> Result<PromiseRaw<'env, PromiseRaw<'env, Object<'env>>>> {
    let store = Rc::clone(&self.store);
    self.hold(core::Side::Reader, || store.begin_read(), handler, move |store, _| store.end_read())
  }
}

impl Space {
  fn lane(&self, side: core::Side) -> &Rc<Lane> {
    match side {
      core::Side::Writer => &self.writer,
      core::Side::Reader => &self.reader,
    }
  }

  // Hold a transaction open on one side across an async handler, ending it when
  // the handler's Promise settles: `end(store, true)` on success, `end(store,
  // false)` on failure, each returning the tables touched. Once it ends, the
  // side's next job starts.
  fn hold<'env>(
    &self,
    side: core::Side,
    begin: impl FnOnce() -> core::Result<()>,
    handler: Function<'env, (), PromiseRaw<'env, Unknown<'env>>>,
    end: impl Fn(&core::Store, bool) -> core::Result<Vec<String>> + Clone + 'env,
  ) -> Result<PromiseRaw<'env, PromiseRaw<'env, Object<'env>>>> {
    let lane = self.lane(side);
    lane.idle()?;
    begin().map_err(to_napi)?;
    lane.open.set(true);

    let pending = match handler.call(()) {
      Ok(pending) => pending,
      // The handler's error is the one the caller needs; ending the transaction can only add noise.
      Err(error) => {
        let _ = end(&self.store, false);
        lane.open.set(false);
        return Err(error);
      }
    };

    // Whether this job's own transaction is still open. By the time the catch
    // step runs, the lane may already hold the next job's transaction.
    let held = Rc::new(Cell::new(true));
    let release = {
      let (store, lane, held) = (Rc::clone(&self.store), Rc::clone(lane), Rc::clone(&held));
      move |env: &Env, commit: bool| -> Result<Option<core::Result<Vec<String>>>> {
        if !held.replace(false) {
          return Ok(None);
        }
        let tables = end(&store, commit);
        lane.open.set(false);
        lane.next(env)?;
        Ok(Some(tables))
      }
    };

    let commit = release.clone();
    let settled = pending.then(move |context| {
      let tables = commit(&context.env, true)?.expect("the transaction is held until the handler settles");
      let mut outcome = Object::new(&context.env)?;
      outcome.set_named_property("value", context.value)?;
      outcome.set_named_property("tables", tables.map_err(to_napi)?)?;
      Ok(outcome)
    })?;

    // Rejecting with the original value keeps a service error intact: napi-rs
    // turns an Err from a promise callback into a fresh Error, dropping its fields.
    settled.catch(move |context: CallbackContext<Unknown<'env>>| {
      release(&context.env, false)?;
      PromiseRaw::<Object<'env>>::reject(&context.env, context.value)
    })
  }
}
