//! One path for every call, whoever makes it and whichever service answers it.
//!
//! A call submitted to a space either reads or writes. A GET reads at once, on
//! its own snapshot of committed state, and never waits. Anything else is an op:
//! stamped as it arrives, written to the op log by the space's OpWriter, and
//! applied in one transaction, which commits its result with its writes. A
//! refused op rolls back and keeps only its result. Either way the call is
//! routed by service name to the service's entry point, which settles it.
//!
//! Which op applies next is read from the log: the first of the pending ops,
//! the op log minus results, in the shared order. They apply one at a time,
//! and only once every service the space names is set up here.
//!
//! A system failure stops the space: the file or disk failed, not the op. It
//! neither logs nor applies another op, and every call to it fails with a 503,
//! until the problem is fixed by hand and the space is resumed. Writing an op
//! to the log or its result is never refused as the op's own failure.

use std::cell::Cell;
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::error::{Result, ServiceError, conflict, not_found, stopped};
use crate::op::actor;
use crate::results::Outcome;
use crate::service_call::{Conn, ServiceCall, Settle};
use crate::services::{self, SERVICE_MISSING, Service};
use crate::space::{RunState, Space};
use crate::store::{Store, tables};
use crate::{Actor, Call, HandlerContext, Op, oplog, results};

// What a call settled with, and the tables it read or wrote, for watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settled {
  pub value: Value,
  pub tables: Vec<String>,
}

// How a submitted call answers whoever submitted it.
pub type Reply = Box<dyn FnOnce(Result<Settled>)>;

impl Carbon {
  // Take a call on a space: a GET reads now, anything else is an op that waits its turn.
  pub fn submit(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    if let Err(error) = accepting(space) {
      return reply(Err(error));
    }
    if !call.writes() {
      return self.read(space, actor, call, input, reply);
    }
    let op = self.stamp(&actor, &call, &input);
    space.wait_for(&op.hash, reply);
    match space.op_writer.write(space.store(), space.is_applying(), op) {
      Ok(()) => self.drain(space),
      Err(error) => self.stop(space, error),
    }
  }

  // Apply a space's pending ops in turn, until one is still applying, none is
  // pending, a service the space names is not set up yet, or it stops.
  pub(crate) fn drain(self: &Rc<Self>, space: &Rc<Space>) {
    if space.draining.replace(true) {
      return;
    }
    while matches!(space.run_state(), RunState::Idle | RunState::Initializing { .. }) {
      if let Err(failure) = between_turns(space) {
        self.stop(space, failure);
        break;
      }
      self.initialize(space);
      if space.run_state() != RunState::Idle {
        break;
      }
      match oplog::next_pending(space.store()) {
        Ok(Some(op)) => self.apply(space, op),
        Ok(None) => break,
        Err(failure) => self.stop(space, failure),
      }
    }
    space.draining.set(false);
  }

  // Move a space between Initializing and Idle: Initializing while a service
  // its origin names is not set up here, with those services.
  fn initialize(&self, space: &Space) {
    let services = space.not_set_up();
    let state = if services.is_empty() { RunState::Idle } else { RunState::Initializing { services } };
    if space.run_state() != state {
      self.set_run_state(space, state);
    }
  }

  // Apply one op: open its transaction and route it. When it settles, commit
  // it with its result, or roll it back and keep only its result.
  fn apply(self: &Rc<Self>, space: &Rc<Space>, op: Op) {
    let call = Call::from_key(&op.key).and_then(|call| self.same_version(space, &call, &op).map(|_| call));
    let call = match call {
      Ok(call) => call,
      Err(error) => return self.refuse(space, &op, error),
    };
    if let Err(failure) = space.store().begin_write() {
      return self.stop(space, failure);
    }
    self.set_run_state(space, RunState::Applying { op: op.hash.clone() });
    let live = Rc::new(Cell::new(true));
    let (carbon, here, ended, applied) = (Rc::clone(self), Rc::clone(space), Rc::clone(&live), op.clone());
    let settle: Settle = Box::new(move |result, calls| {
      ended.set(false);
      carbon.finish(&here, &applied, result, json!(calls));
      carbon.drain(&here);
    });
    let service_call = ServiceCall::new(self, space, actor(&op), call, op.input.clone(), Conn::Writer, Some(op.clone()), live, settle);
    self.route(service_call);
  }

  // An op settled: end its transaction with its result and answer whoever waits on it.
  fn finish(&self, space: &Space, op: &Op, result: Result<Value>, calls: Value) {
    match end(space.store(), op, result, calls) {
      Ok(settled) => {
        self.set_run_state(space, RunState::Idle);
        if let Ok(settled) = &settled {
          self.committed(&space.id, &settled.tables);
        }
        space.answer(&op.hash, settled);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // Refuse an op before it applies, keeping its result.
  fn refuse(&self, space: &Space, op: &Op, error: ServiceError) {
    match results::record(space.store(), &op.hash, &Outcome::Refused { error: error.clone(), calls: json!([]) }) {
      Ok(()) => space.answer(&op.hash, Err(error)),
      Err(failure) => self.stop(space, failure),
    }
  }

  // Stop a space after a system failure: hold nothing more for the log, and
  // answer everyone waiting on its ops with the failure.
  pub(crate) fn stop(&self, space: &Space, failure: ServiceError) {
    space.op_writer.discard();
    self.set_run_state(space, RunState::Stopped { error: failure.clone() });
    space.answer_all(&stopped(&space.id, &failure));
  }

  // Resume a stopped space once its failure is fixed: its pending ops apply
  // again from the first, the op that was applying when it stopped included.
  pub fn resume(self: &Rc<Self>, space: &Rc<Space>) {
    if !matches!(space.run_state(), RunState::Stopped { .. }) {
      return;
    }
    space.store().clear_failure();
    self.set_run_state(space, RunState::Idle);
    self.drain(space);
  }

  // An op applies only with the version of the service it was made for.
  fn same_version(&self, space: &Space, call: &Call, op: &Op) -> Result<()> {
    if op.version == SERVICE_MISSING {
      return Err(not_found(format!("[{}] was not registered on this device when {} was made", call.service, op.key)).with_details(json!({"service": call.service})));
    }
    let service = answering(self, space, call)?;
    if service.version == op.version {
      return Ok(());
    }
    Err(conflict(format!("{} was made with version {} of [{}], and this device runs version {}", op.key, op.version, service.name, service.version)))
  }

  // Run a GET on its own snapshot of committed state. A system failure met on
  // the way stops the space, as an op's would.
  fn read(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    let reader = match space.store().reader() {
      Ok(reader) => Rc::new(reader),
      Err(error) => return self.answer_read(space, reply, Err(error)),
    };
    let live = Rc::new(Cell::new(true));
    let (carbon, here, snapshot, ended) = (Rc::clone(self), Rc::clone(space), Rc::clone(&reader), Rc::clone(&live));
    let settle: Settle = Box::new(move |result, _calls| {
      ended.set(false);
      let tables = snapshot.end();
      carbon.answer_read(&here, reply, result.and_then(|value| Ok(Settled { value, tables: tables? })));
    });
    self.route(ServiceCall::new(self, space, actor, call, input, Conn::Reader(reader), None, live, settle));
  }

  fn answer_read(&self, space: &Space, reply: Reply, settled: Result<Settled>) {
    if let Some(failure) = space.store().failure() {
      self.stop(space, failure);
    }
    reply(settled)
  }

  // Hand a call to the entry point of the service that answers it here.
  pub(crate) fn route(&self, service_call: Rc<ServiceCall>) {
    match answering(self, service_call.space(), &service_call.call) {
      Ok(service) => (service.entry)(service_call),
      Err(error) => {
        let _ = service_call.settle(Err(error));
      }
    }
  }
}

// The service a call reaches, by name: a local service this device runs, or a
// replicated one set up in this space. The root space runs local services only.
fn answering(carbon: &Carbon, space: &Space, call: &Call) -> Result<Rc<Service>> {
  let found = carbon.services().find(&call.service).filter(|service| service.local == call.confined);
  match (call.confined, found) {
    (true, Some(service)) => Ok(service),
    (true, None) => Err(not_found(format!("No local service [@{}] on this device", call.service))),
    (false, _) if space.is_root() => Err(not_found(format!("The system handle answers only local services, not {}", call.key()))),
    (false, Some(service)) if space.runs(&call.service) => Ok(service),
    (false, None) if space.names(&call.service) => Err(services::missing(&call.service)),
    (false, _) => Err(not_found(format!("[{}] does not run in this space", call.service))),
  }
}

// Calls to a stopped space fail with the failure that stopped it.
fn accepting(space: &Space) -> Result<()> {
  match space.run_state() {
    RunState::Stopped { error } => Err(stopped(&space.id, &error)),
    _ => Ok(()),
  }
}

// Between two ops: write the ops that arrived while the last applied, and set
// up the services that waited for it. A service whose set-up fails stays
// unset, which holds the space's ops back until it is registered again.
fn between_turns(space: &Rc<Space>) -> Result<()> {
  space.op_writer.write_held(space.store())?;
  let _ = space.set_up_awaiting();
  space.store().failure().map_or(Ok(()), Err)
}

// End an op's transaction: commit it with its result when it succeeded, or
// roll it back and keep only its result, saying why. Fails, leaving the op
// pending, when the store met a system failure or the result could not be
// written: then the space stops.
fn end(store: &Store, op: &Op, result: Result<Value>, calls: Value) -> Result<Result<Settled>> {
  // Rolling back while stopping: the space stops with the first failure,
  // whatever the rollback meets.
  if let Some(failure) = store.failure() {
    let _ = store.end_write(false);
    return Err(failure);
  }
  match result {
    Ok(value) => {
      let applied = Outcome::Applied { output: value.clone(), calls };
      if let Err(failure) = results::record(store, &op.hash, &applied) {
        let _ = store.end_write(false);
        return Err(failure);
      }
      let changeset = store.end_write(true)?;
      Ok(Ok(Settled { value, tables: tables(&changeset)? }))
    }
    Err(error) => {
      store.end_write(false)?;
      results::record(store, &op.hash, &Outcome::Refused { error: error.clone(), calls })?;
      Ok(Err(error))
    }
  }
}
