//! One path for every call, whoever makes it and whichever service answers it.
//!
//! A call submitted to a space either reads or writes. A GET reads on its own
//! snapshot of committed state: at once, or, while its space initializes,
//! once it is ready (`crate::lifecycle`). Anything else is an op:
//! stamped as it arrives, written to the op log by the space's OpWriter, and
//! applied in one transaction, which commits its result with its writes. A
//! refused op rolls back and keeps only its result. Either way the call is
//! routed by service name to the service's entry point, which settles it.
//!
//! Every call it is handed must settle within its service's timeout. One
//! deadline, armed when the call is taken (`crate::deadline`), covers the
//! call wherever it waits and every call it makes inside it; when it passes,
//! the core settles the call with a timeout, rolling an op back like any other
//! failure, refusing one still waiting in the log, and dropping whatever its
//! handler answers later.
//!
//! Which op applies next is read from the log: the first of the pending ops,
//! the op log minus results, in the shared order. They apply one at a time,
//! and only once the space is ready. An op that sorts before
//! ops already applied is late: it is tried in place, and when that run would
//! not match applying it at its place the space rewinds (`rewind`). An op's
//! first run makes its effects and records them on its row; every later run,
//! here after a rewind or on another device, replays them (`ops::effects`).
//!
//! A system failure stops the space: the file or disk failed, not the op. It
//! neither logs nor applies another op, and every call to it fails with a 503,
//! until the problem is fixed by hand and the space is resumed. So does a
//! space that cannot become ready (`crate::lifecycle`). Writing an op
//! to the log or its result is never refused as the op's own failure.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::deadline::Deadline;
use crate::error::{Result, ServiceError, conflict, not_found, stopped};
use crate::ops::effects::{EffectLog, Made};
use crate::ops::op::actor;
use crate::ops::oplog::LogRow;
use crate::ops::results::Outcome;
use crate::sql::authorizer::Read;
use crate::ops::rewind::{self, Recorded, Sets};
use crate::service_call::{CallRecord, Conn, ServiceCall, Settle};
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

// A call the core has taken: who makes it, what it asks, the spaces whose
// ops wait on it, the deadline it must settle by, and where it answers.
pub(crate) struct Taken {
  pub(crate) actor: Actor,
  pub(crate) call: Call,
  pub(crate) input: Value,
  pub(crate) awaiting: Vec<String>,
  pub(crate) deadline: Rc<Deadline>,
  pub(crate) reply: Reply,
}

impl Carbon {
  // Take a call on a space: a GET reads once the space is ready, anything
  // else is an op that waits its turn.
  pub fn submit(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    self.post(space, actor, call, input, vec![], reply)
  }

  // Take a call that the ops of the spaces in `awaiting` wait on, under a
  // deadline that begins once the space is ready, which answering it cancels.
  pub(crate) fn post(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, awaiting: Vec<String>, reply: Reply) {
    let deadline = Deadline::new(call.key(), self.timeout_of(space));
    let answering = Rc::clone(&deadline);
    let reply: Reply = Box::new(move |settled| {
      answering.answered();
      reply(settled)
    });
    self.take(space, Taken { actor, call, input, awaiting, deadline, reply })
  }

  // A call taken: refused by a stopped space; a GET held while its space
  // initializes and read once it is ready; an op queued in the log. Its
  // deadline begins now, unless its space is still starting.
  pub(crate) fn take(self: &Rc<Self>, space: &Rc<Space>, taken: Taken) {
    if let Err(error) = accepting(space) {
      return (taken.reply)(Err(error));
    }
    if !matches!(space.run_state(), RunState::Initializing { .. }) {
      taken.deadline.begin(self);
    }
    match (taken.call.writes(), space.run_state()) {
      (true, _) => self.queue(space, taken),
      (false, RunState::Initializing { .. }) => self.hold(space, taken),
      (false, _) => self.read(space, taken),
    }
  }

  // Log an op to wait its turn; should its caller's deadline pass first, the
  // caller is answered and the op refused when its turn comes.
  fn queue(self: &Rc<Self>, space: &Rc<Space>, taken: Taken) {
    let Taken { actor, call, input, awaiting, deadline, reply } = taken;
    let op = self.stamp(&actor, &call, &input, &space.runs.package);
    let (here, hash) = (Rc::clone(space), op.hash.clone());
    deadline.waiting(Box::new(move |error| here.give_up(&hash, error)));
    space.wait_for(&op.hash, awaiting, reply, deadline);
    self.log(space, LogRow::new(op));
  }

  // The timeout a call to a space must settle within: its service's, once
  // this device has its package, compiled or compiling; the default before.
  fn timeout_of(&self, space: &Space) -> std::time::Duration {
    space.service().or_else(|| self.services().find(&space.runs.package)).map_or(services::DEFAULT_TIMEOUT, |service| service.timeout)
  }

  // Write a row to a space's log, then apply what is pending.
  pub(crate) fn log(self: &Rc<Self>, space: &Rc<Space>, row: LogRow) {
    match space.op_writer.write(space.store(), space.is_applying(), row) {
      Ok(()) => self.drain(space),
      Err(error) => self.stop(space, error),
    }
  }

  // Apply a space's pending ops in turn, until one is still applying, none is
  // pending, or the space is not ready.
  pub(crate) fn drain(self: &Rc<Self>, space: &Rc<Space>) {
    if space.draining.replace(true) {
      return;
    }
    while space.run_state() == RunState::Idle {
      if let Err(failure) = between_turns(space) {
        self.stop(space, failure);
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

  // Apply one op, with the version of its service it was made for: a first
  // run, or a replay of the effects its row carries. An op its caller gave
  // up on is refused with what the caller got.
  fn apply(self: &Rc<Self>, space: &Rc<Space>, row: LogRow) {
    if let Some(expired) = space.expired(&row.op.hash) {
      return self.refuse(space, &row.op, expired);
    }
    let call = match Call::from_key(&row.op.key) {
      Ok(call) => call,
      Err(error) => return self.refuse(space, &row.op, error),
    };
    match self.same_version(space, &call, &row.op) {
      Ok(()) => {
        let deadline = space.deadline_of(&row.op.hash);
        if let Some(service_call) = self.begin(space, row.op, call, row.effects) {
          self.start(service_call, deadline);
        }
      }
      Err(error) => self.refuse(space, &row.op, error),
    }
  }

  // Open an op's transaction, and the service call it runs as, which settles
  // it. The ops after it in the shared order are read first: when there are
  // any, the op is late, and is tried in place.
  fn begin(self: &Rc<Self>, space: &Rc<Space>, op: Op, call: Call, recorded: Option<Vec<Made>>) -> Option<Rc<ServiceCall>> {
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
    let effects = Rc::new(RefCell::new(EffectLog::of(recorded)));
    let (carbon, here, ended, applied, kept) = (Rc::clone(self), Rc::clone(space), Rc::clone(&live), op.clone(), Rc::clone(&effects));
    let settle: Settle = Box::new(move |result, calls| {
      ended.set(false);
      let made = kept.borrow().made().map(<[Made]>::to_vec);
      carbon.finish(&here, &applied, &later, result, calls, made);
      carbon.drain(&here);
    });
    let awaiting = space.awaiting(&op.hash);
    Some(ServiceCall::new(self, space, actor(&op), call, op.input.clone(), Conn::Writer, Some(op), effects, awaiting, live, settle))
  }

  // An op settled. A late op whose run would not match applying it at its
  // place rolls back and rewinds the space; any other ends its transaction
  // with its result and answers whoever waits on it.
  fn finish(self: &Rc<Self>, space: &Rc<Space>, op: &Op, later: &[Recorded], result: Result<Value>, calls: Vec<CallRecord>, made: Option<Vec<Made>>) {
    let ran = Ran::of(space.store(), result.is_ok());
    let ran = match ran {
      Ok(ran) => ran,
      Err(failure) => return self.stop(space, failure),
    };
    if !rewind::fits_in_place(&ran.sets, later) {
      return self.rewind(space, op, made, later);
    }
    match end(space.store(), op, &ran, result, calls, made.as_deref()).and_then(|settled| self.unmake_refused(&settled, made.as_deref()).map(|_| settled)) {
      Ok(settled) => {
        self.set_run_state(space, RunState::Idle);
        let settled = settled.map(|settled| Settled { watches: self.committed(&space.id, &settled.tables), ..settled });
        space.answer(&op.hash, settled);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // A refused op's first run leaves nothing behind: the spaces it created are deleted.
  fn unmake_refused(self: &Rc<Self>, settled: &Result<Settled>, made: Option<&[Made]>) -> Result<()> {
    match (settled, made) {
      (Err(_), Some(made)) => self.unmake(made),
      _ => Ok(()),
    }
  }

  // Roll back a late op's run, then undo the later ops that touched the
  // space's tables, and forget their results, in one transaction: they are
  // pending again, so the late op applies next at its place, and they re-run
  // after it. Effects its first run made are kept on its row, so its run at
  // its place replays them rather than making them again.
  fn rewind(self: &Rc<Self>, space: &Rc<Space>, op: &Op, made: Option<Vec<Made>>, later: &[Recorded]) {
    if let Err(failure) = space.store().end_write(false) {
      return self.stop(space, failure);
    }
    self.set_run_state(space, RunState::Idle);
    let undone = rewind::undo_set(later);
    let hashes: Vec<String> = undone.iter().map(|op| op.hash.clone()).collect();
    let undo = space.store().write(|store| {
      undone.iter().try_for_each(|op| store.undo(&op.changeset))?;
      made.iter().try_for_each(|made| oplog::record_effects(store, &op.hash, made))?;
      results::forget(store, &hashes)
    });
    match undo.and_then(|(_, changeset)| tables(&changeset)) {
      Ok(tables) => {
        self.committed(&space.id, &tables);
      }
      Err(failure) => self.stop(space, failure),
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
    self.release(&space.id);
  }

  // Resume a stopped space once its failure is fixed: one never set up
  // initializes again; any other applies its pending ops again from the
  // first, the op that was applying when it stopped included. Returns the
  // watches it ran again.
  pub fn resume(self: &Rc<Self>, space: &Rc<Space>) -> Vec<WatchId> {
    if !matches!(space.run_state(), RunState::Stopped { .. }) {
      return vec![];
    }
    space.store().clear_failure();
    if space.service().is_none() {
      return self.initialize(space);
    }
    let rerun = self.set_run_state(space, RunState::Idle);
    self.drain(space);
    rerun
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

  // Run a GET on its own snapshot of committed state, under its deadline. A
  // system failure met on the way stops the space, as an op's would.
  pub(crate) fn read(self: &Rc<Self>, space: &Rc<Space>, taken: Taken) {
    let Taken { actor, call, input, awaiting, deadline, reply } = taken;
    let reader = match space.store().reader() {
      Ok(reader) => Rc::new(reader),
      Err(error) => return self.answer_read(space, reply, Err(error)),
    };
    let live = Rc::new(Cell::new(true));
    let effects = Rc::new(RefCell::new(EffectLog::of(None)));
    let (carbon, here, snapshot, ended) = (Rc::clone(self), Rc::clone(space), Rc::clone(&reader), Rc::clone(&live));
    let settle: Settle = Box::new(move |result, _calls| {
      ended.set(false);
      let tables = snapshot.end();
      carbon.answer_read(&here, reply, result.and_then(|value| Ok(Settled { value, tables: tables?, watches: vec![] })));
    });
    self.start(ServiceCall::new(self, space, actor, call, input, Conn::Reader(reader), None, effects, awaiting, live, settle), Some(deadline));
  }

  fn answer_read(self: &Rc<Self>, space: &Space, reply: Reply, settled: Result<Settled>) {
    if let Some(failure) = space.store().failure() {
      self.stop(space, failure);
    }
    reply(settled)
  }

  // Hand a call to its service, under its caller's deadline, or, for an op
  // no caller waits on, one its service's timeout sets now. The deadline
  // keeps the call until it settles, so a handler that drops it is still
  // settled.
  fn start(&self, service_call: Rc<ServiceCall>, deadline: Option<Rc<Deadline>>) {
    if let Ok(service) = answering(self, service_call.space(), &service_call.call) {
      let deadline = deadline.unwrap_or_else(|| Deadline::arm(self, service_call.call.key(), service.timeout));
      service_call.set_deadline(deadline);
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

// The service a call reaches: the space's own, set up here, by the name it
// goes by. The system space runs `[@carbon]` only.
fn answering(_carbon: &Carbon, space: &Space, call: &Call) -> Result<Rc<Service>> {
  match (space.answers(call), space.service()) {
    (true, Some(service)) => Ok(service),
    (true, None) => Err(services::missing(&space.runs.package)),
    (false, _) if space.is_root() => Err(not_found(format!("The system space runs [@carbon] only, not {}", call.key()))),
    (false, _) => Err(not_found(format!("{} is not this space's service, [{}]", call.key(), space.runs.name))),
  }
}

// The `call` effect: a call to another service, posted to its space as the
// caller's actor, or, for a local service, as the actor it names. The space
// is the one named, or the one the caller's channel names for the call's
// service; a channel's space answers for the services it names. A write into
// a space whose op waits on this call, directly or through other calls, would
// wait behind that op until the timeout, so it is refused at once.
pub(crate) fn call_effect(caller: Rc<ServiceCall>, input: Value, reply: crate::Reply) {
  let carbon = Rc::clone(caller.carbon());
  let made = Call::from_key(input["key"].as_str().unwrap_or_default()).and_then(|call| {
    let actor = match input.get("as").filter(|_| caller.call.confined) {
      Some(actor) => serde_json::from_value(actor.clone())?,
      None => caller.actor.clone(),
    };
    let space = match input["space"].as_str() {
      Some(space) => space.to_string(),
      None => carbon.channel_of(caller.space())?,
    };
    Ok((call, actor, space))
  });
  let (call, actor, space) = match made {
    Ok(made) => made,
    Err(error) => return reply(Err(error)),
  };
  let awaiting: Vec<String> = caller.awaiting().iter().cloned().chain(std::iter::once(caller.space().id.clone())).collect();
  let target = carbon.service_space(&space, &call).and_then(|target| match call.writes() && awaiting.contains(&target.id) {
    true => Err(looped(&call)),
    false => Ok(target),
  });
  match target {
    Ok(target) => carbon.post(&target, actor, call, input["input"].clone(), awaiting, Box::new(move |settled| reply(settled.map(|settled| settled.value)))),
    Err(error) => reply(Err(error)),
  }
}

// A write posted back into a service whose op waits on the call posting it.
fn looped(call: &Call) -> ServiceError {
  ServiceError::new(508, "LOOP_DETECTED", format!("{} writes to a service whose op is waiting on this call, which would wait for itself", call.key()))
}

// Calls to a stopped space fail with the failure that stopped it, and so do rows received for it.
pub(crate) fn accepting(space: &Space) -> Result<()> {
  match space.run_state() {
    RunState::Stopped { error } => Err(stopped(&space.id, &error)),
    _ => Ok(()),
  }
}

// Between two ops: write the ops that arrived while the last applied.
fn between_turns(space: &Rc<Space>) -> Result<()> {
  space.op_writer.write_held(space.store())?;
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

// End an op's transaction: commit it with its result when it succeeded, or
// roll it back and keep only its result, saying why. Either result keeps the
// op's read set and changeset, and on its first run the effects it made.
// Fails, leaving the op pending, when the store met a system failure or the
// result could not be written: then the space stops.
fn end(store: &Store, op: &Op, ran: &Ran, result: Result<Value>, calls: Vec<CallRecord>, made: Option<&[Made]>) -> Result<Result<Settled>> {
  // Rolling back while stopping: the space stops with the first failure,
  // whatever the rollback meets.
  if let Some(failure) = store.failure() {
    let _ = store.end_write(false);
    return Err(failure);
  }
  match result {
    Ok(value) => {
      let applied = Outcome::Applied { output: value.clone(), calls: json!(calls) };
      let recorded = results::record(store, &op.hash, &applied, &ran.read_set, &ran.changeset).and_then(|_| made.iter().try_for_each(|made| oplog::record_effects(store, &op.hash, made)));
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
      made.iter().try_for_each(|made| oplog::record_effects(store, &op.hash, made))?;
      Ok(Err(error))
    }
  }
}
