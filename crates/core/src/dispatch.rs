//! One path for every call, whoever makes it and whichever service answers it.
//!
//! A call submitted to a space either reads or writes. A GET reads at once, on
//! its own snapshot of committed state, and never waits. Anything else is an op:
//! stamped as it arrives, written to the op log by the space's OpWriter, and
//! applied in one transaction, which commits its result with its writes. A
//! refused op rolls back and keeps only its result. Either way the call is
//! routed by service name to the service's entry point, which settles it.
//!
//! Every call it is handed must settle within its service's timeout. One
//! deadline covers a call and every call it makes inside it; when it passes,
//! the core settles the call with a timeout, rolling an op back like any other
//! failure, and drops whatever its handler answers later.
//!
//! Which op applies next is read from the log: the first of the pending ops,
//! the op log minus results, in the shared order. They apply one at a time,
//! and only once every service the space names is set up here. An op that
//! sorts before ops already applied is late: it is tried in place, and when
//! that run would not match applying it at its place the space rewinds
//! (`rewind`). A local op runs once, on the device that made it; every replay
//! of it, here after a rewind or on another device, runs the sub-calls its row
//! carries instead.
//!
//! A system failure stops the space: the file or disk failed, not the op. It
//! neither logs nor applies another op, and every call to it fails with a 503,
//! until the problem is fixed by hand and the space is resumed. Writing an op
//! to the log or its result is never refused as the op's own failure.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::error::{Result, ServiceError, conflict, not_found, stopped};
use crate::ops::op::actor;
use crate::ops::oplog::LogRow;
use crate::ops::results::Outcome;
use crate::sql::authorizer::Read;
use crate::ops::rewind::{self, Recorded, Sets};
use crate::service_call::{CallRecord, Conn, ServiceCall, Settle, sub_calls};
use crate::services::{self, SERVICE_MISSING, Service};
use crate::space::{RunState, Space};
use crate::store::Store;
use crate::store::changeset::tables;
use crate::watch::WatchId;
use crate::ops::{oplog, results};
use crate::{Actor, Call, Routing, HandlerContext, Op};

// What a call settled with; the tables it read or wrote, for watch; and the
// watches its commit owes a run, which a call from the app waits for. A GET
// commits nothing, so it owes none.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settled {
  pub value: Value,
  pub tables: Vec<String>,
  pub watches: Vec<WatchId>,
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
    self.log(space, LogRow::new(op));
  }

  // Write a row to a space's log, then apply what is pending.
  pub(crate) fn log(self: &Rc<Self>, space: &Rc<Space>, row: LogRow) {
    match space.op_writer.write(space.store(), space.is_applying(), row) {
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
        Ok(Some(row)) => self.apply(space, row),
        Ok(None) => break,
        Err(failure) => self.stop(space, failure),
      }
    }
    space.draining.set(false);
  }

  // Move a space between Initializing and Idle: Initializing while a service
  // its origin names is not set up here, with those services.
  fn initialize(self: &Rc<Self>, space: &Space) {
    let services = space.not_set_up();
    let state = if services.is_empty() { RunState::Idle } else { RunState::Initializing { services } };
    if space.run_state() != state {
      self.set_run_state(space, state);
    }
  }

  // Apply one op: a local op that has run replays its sub-calls; any other
  // op runs, with the version of its service it was made for.
  fn apply(self: &Rc<Self>, space: &Rc<Space>, row: LogRow) {
    let call = match Call::from_key(&row.op.key) {
      Ok(call) => call,
      Err(error) => return self.refuse(space, &row.op, error),
    };
    match (call.confined, row.sub_calls) {
      (true, Some(sub_calls)) => self.replay(space, row.op, call, sub_calls),
      _ => match self.same_version(space, &call, &row.op) {
        Ok(()) => self.run(space, row.op, call),
        Err(error) => self.refuse(space, &row.op, error),
      },
    }
  }

  // Run an op through its service's entry point, in its transaction.
  fn run(self: &Rc<Self>, space: &Rc<Space>, op: Op, call: Call) {
    if let Some(service_call) = self.begin(space, op, call) {
      self.start(service_call);
    }
  }

  // Replay a local op that has run: its recorded sub-calls, in its
  // transaction, never the local op itself. One deadline covers them all,
  // each sub-call's service granting its own timeout.
  fn replay(self: &Rc<Self>, space: &Rc<Space>, op: Op, call: Call, sub_calls: Vec<Op>) {
    let timeout = sub_calls.iter().filter_map(|sub_call| Call::from_key(&sub_call.key).ok()).filter_map(|call| self.services().find(&call.service)).map(|service| service.timeout).sum::<Duration>();
    let Some(local) = self.begin(space, op, call) else { return };
    let expiring = Rc::clone(&local);
    local.set_deadline(self.after(timeout, Box::new(move || expiring.expire(timeout))));
    self.replay_sub_calls(local, VecDeque::from(sub_calls), vec![]);
  }

  // Run a local op's recorded sub-calls in order, each under its own
  // savepoint, so one refused on replay rolls back only its own writes; then
  // settle the local op with what each answered.
  fn replay_sub_calls(self: &Rc<Self>, local: Rc<ServiceCall>, mut sub_calls: VecDeque<Op>, mut answers: Vec<Value>) {
    let Some(sub_call) = sub_calls.pop_front() else {
      let _ = local.settle(Ok(json!(answers)));
      return;
    };
    let (carbon, next) = (Rc::clone(self), Rc::clone(&local));
    let reply: crate::Reply = Box::new(move |result| {
      answers.push(match result {
        Ok(output) => json!({"output": output}),
        Err(error) => json!({"error": error}),
      });
      carbon.replay_sub_calls(next, sub_calls, answers)
    });
    match Call::from_key(&sub_call.key).and_then(|call| self.same_version(local.space(), &call, &sub_call)) {
      Ok(()) => local.replay(sub_call, reply),
      Err(error) => reply(Err(error)),
    }
  }

  // Open an op's transaction, and the service call it runs as, which settles
  // it. The ops after it in the shared order are read first: when there are
  // any, the op is late, and is tried in place.
  fn begin(self: &Rc<Self>, space: &Rc<Space>, op: Op, call: Call) -> Option<Rc<ServiceCall>> {
    let later = match rewind::after(space.store(), &op) {
      Ok(later) => later,
      Err(failure) => {
        self.stop(space, failure);
        return None;
      }
    };
    if let Err(failure) = space.store().begin_write() {
      self.stop(space, failure);
      return None;
    }
    self.set_run_state(space, RunState::Applying { op: op.hash.clone() });
    let live = Rc::new(Cell::new(true));
    let (carbon, here, ended, applied, local) = (Rc::clone(self), Rc::clone(space), Rc::clone(&live), op.clone(), call.confined);
    let settle: Settle = Box::new(move |result, calls| {
      ended.set(false);
      carbon.finish(&here, &applied, local, &later, result, calls);
      carbon.drain(&here);
    });
    Some(ServiceCall::new(self, space, actor(&op), call, op.input.clone(), Conn::Writer, Some(op), live, settle))
  }

  // An op settled. A late op whose run would not match applying it at its
  // place rolls back and rewinds the space; any other ends its transaction
  // with its result and answers whoever waits on it.
  fn finish(self: &Rc<Self>, space: &Rc<Space>, op: &Op, local: bool, later: &[Recorded], result: Result<Value>, calls: Vec<CallRecord>) {
    let ran = Ran::of(space.store(), result.is_ok());
    let ran = match ran {
      Ok(ran) => ran,
      Err(failure) => return self.stop(space, failure),
    };
    if !rewind::fits_in_place(&ran.sets, later) {
      return self.rewind(space, &ran.sets, later);
    }
    match end(space.store(), op, local, &ran, result, calls) {
      Ok(settled) => {
        self.set_run_state(space, RunState::Idle);
        self.learn_origin(space);
        let settled = settled.map(|settled| Settled { watches: self.committed(&space.id, &settled.tables), ..settled });
        space.answer(&op.hash, settled);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // Roll back a late op's run, then undo the later ops it depends on or that
  // depend on it, and forget their results, in one transaction: they are
  // pending again, so the late op applies next at its place, and they re-run
  // after it.
  fn rewind(self: &Rc<Self>, space: &Rc<Space>, sets: &Sets, later: &[Recorded]) {
    if let Err(failure) = space.store().end_write(false) {
      return self.stop(space, failure);
    }
    self.set_run_state(space, RunState::Idle);
    let undone = rewind::undo_set(sets, later);
    let hashes: Vec<String> = undone.iter().map(|op| op.hash.clone()).collect();
    let undo = space.store().write(|store| {
      undone.iter().try_for_each(|op| store.undo(&op.changeset))?;
      results::forget(store, &hashes)
    });
    match undo.and_then(|(_, changeset)| tables(&changeset)) {
      Ok(tables) => {
        self.committed(&space.id, &tables);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // Once a new space's origin applies, learn the services it names and set
  // up those this device runs. A service whose set-up fails stays unset,
  // which holds the space's ops back, as on open.
  fn learn_origin(self: &Rc<Self>, space: &Rc<Space>) {
    if !space.is_root() && !space.knows_origin() {
      self.learn(space, Box::new(|_| {}));
    }
  }

  // Refuse an op before it applies, keeping its result.
  fn refuse(self: &Rc<Self>, space: &Space, op: &Op, error: ServiceError) {
    match results::record(space.store(), &op.hash, &Outcome::Refused { error: error.clone(), calls: json!([]) }, &[], &[]) {
      Ok(()) => space.answer(&op.hash, Err(error)),
      Err(failure) => self.stop(space, failure),
    }
  }

  // Stop a space after a system failure: hold nothing more for the log, and
  // answer everyone waiting on its ops with the failure.
  pub(crate) fn stop(self: &Rc<Self>, space: &Space, failure: ServiceError) {
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
      carbon.answer_read(&here, reply, result.and_then(|value| Ok(Settled { value, tables: tables?, watches: vec![] })));
    });
    self.start(ServiceCall::new(self, space, actor, call, input, Conn::Reader(reader), None, live, settle));
  }

  fn answer_read(self: &Rc<Self>, space: &Space, reply: Reply, settled: Result<Settled>) {
    if let Some(failure) = space.store().failure() {
      self.stop(space, failure);
    }
    reply(settled)
  }

  // Hand a call to its service, under the deadline its service's timeout sets.
  // The timer keeps the call until it settles, so a handler that drops it is
  // still settled.
  fn start(&self, service_call: Rc<ServiceCall>) {
    if let Ok(service) = answering(self, service_call.space(), &service_call.call) {
      let (expiring, timeout) = (Rc::clone(&service_call), service.timeout);
      service_call.set_deadline(self.after(timeout, Box::new(move || expiring.expire(timeout))));
    }
    self.route(service_call);
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
    (false, _) if space.is_root() => Err(not_found(format!("The system space runs local services only, not {}", call.key()))),
    (false, Some(service)) if space.runs(&call.service) => Ok(service),
    (false, None) if space.names(&call.service) => Err(services::missing(&call.service)),
    (false, _) => Err(not_found(format!("[{}] does not run in this space", call.service))),
  }
}

// Calls to a stopped space fail with the failure that stopped it, and so do rows received for it.
pub(crate) fn accepting(space: &Space) -> Result<()> {
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

// What an op's run did, read from its open transaction: its read set, and
// its changeset, empty when it is refused and its writes roll back.
struct Ran {
  read_set: Vec<Read>,
  changeset: Vec<u8>,
  sets: Sets,
}

impl Ran {
  fn of(store: &Store, applied: bool) -> Result<Ran> {
    let read_set = store.read_set();
    let changeset = if applied { store.changeset()? } else { vec![] };
    let sets = Sets::of(&read_set, &changeset)?;
    Ok(Ran { read_set, changeset, sets })
  }
}

// End an op's transaction: commit it with its result, and a local op on its
// first run with its sub-calls, when it succeeded; or roll it back and keep
// only its result, saying why. Either result keeps the op's read set and
// changeset. Fails, leaving the op pending, when the store met a system
// failure or the result could not be written: then the space stops.
fn end(store: &Store, op: &Op, local: bool, ran: &Ran, result: Result<Value>, calls: Vec<CallRecord>) -> Result<Result<Settled>> {
  // Rolling back while stopping: the space stops with the first failure,
  // whatever the rollback meets.
  if let Some(failure) = store.failure() {
    let _ = store.end_write(false);
    return Err(failure);
  }
  match result {
    Ok(value) => {
      let applied = Outcome::Applied { output: value.clone(), calls: json!(calls) };
      let recorded = results::record(store, &op.hash, &applied, &ran.read_set, &ran.changeset).and_then(|_| match local {
        true => oplog::record_sub_calls(store, &op.hash, &sub_calls(&calls)),
        false => Ok(()),
      });
      if let Err(failure) = recorded {
        let _ = store.end_write(false);
        return Err(failure);
      }
      store.end_write(true)?;
      Ok(Ok(Settled { value, tables: ran.sets.written().into_iter().collect(), watches: vec![] }))
    }
    Err(error) => {
      store.end_write(false)?;
      results::record(store, &op.hash, &Outcome::Refused { error: error.clone(), calls: json!(calls) }, &ran.read_set, &[])?;
      Ok(Err(error))
    }
  }
}
