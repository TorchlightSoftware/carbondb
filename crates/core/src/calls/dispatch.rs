//! One path for every call, whoever makes it and whichever service answers it.
//!
//! A call submitted to a service's store either reads or writes. A GET reads
//! on its own snapshot of committed state: at once, or, while its store
//! initializes, once it is ready (`crate::channels::lifecycle`). Anything else
//! is an op: stamped as it arrives, written to its channel's log, pointed to
//! by the store's OpWriter, and applied in one transaction, which commits its
//! result with its writes. A refused op rolls back and keeps only its result.
//! Either way the call is routed by service name to the service's entry
//! point, which settles it. What became of each op is also its outcome in the
//! channel's log, recorded with its effects before its transaction commits.
//!
//! Every call it is handed must settle within its service's timeout. One
//! deadline, armed when the call is taken (`crate::calls::deadline`), covers the
//! call wherever it waits and every call it makes inside it; when it passes,
//! the core settles the call with a timeout, rolling an op back like any other
//! failure, refusing one still waiting in the log, and dropping whatever its
//! handler answers later.
//!
//! Which op applies next is read from the store's pointers: the first of the
//! pending ops, the pointers minus results, in the shared order. They apply
//! one at a time, and only once the store is ready. An op that sorts before
//! ops already applied is late: it is tried in place, and when that run
//! would not match applying it at its place the store rewinds (`rewind`). An
//! op's first run makes its effects and records them on its row; every later
//! run, here after a rewind or on another device, replays them
//! (`ops::effects`). A refused op's first run deletes what its effects
//! created.
//!
//! An op made by another op's call is bound to it, its cause: it is refused
//! once its cause has failed, and when what became of an op changes, the ops
//! its calls made are taken up again, undone where they applied
//! (`reconsider`), so a cause's calls follow it on every replica.
//!
//! A system failure stops the space: the file or disk failed, not the op. It
//! neither logs nor applies another op, and every call to it fails with a 503,
//! until the problem is fixed by hand and the space is resumed. So does a
//! space that cannot become ready (`crate::channels::lifecycle`). Writing an op
//! to the log or its result is never refused as the op's own failure.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::calls::deadline::Deadline;
use crate::error::{Result, ServiceError, conflict, not_found, stopped};
use crate::ops::effects::{EffectLog, Made};
use crate::ops::op::actor;
use crate::ops::oplog::LogRow;
use crate::ops::results::Outcome;
use crate::sql::authorizer::Read;
use crate::ops::rewind::{self, Recorded, Sets};
use crate::calls::service_call::{CallRecord, Conn, ServiceCall, Settle};
use crate::services::{self, SERVICE_MISSING, Service};
use crate::channels::service::{RunState, ServiceStore};
use crate::store::Store;
use crate::store::changeset::tables;
use crate::calls::watch::WatchId;
use crate::ops::{oplog, pointers, results};
use crate::channels::{cause_failed, pointer_to};
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

// A call the core has taken: who makes it, what it asks, the op whose call
// made it, if one did, the stores whose ops wait on it, the deadline it must
// settle by, and where it answers.
pub(crate) struct Taken {
  pub(crate) actor: Actor,
  pub(crate) call: Call,
  pub(crate) input: Value,
  pub(crate) cause: Option<String>,
  pub(crate) awaiting: Vec<String>,
  pub(crate) deadline: Rc<Deadline>,
  pub(crate) reply: Reply,
}

impl Carbon {
  // Take a call on a space: a GET reads once the space is ready, anything
  // else is an op that waits its turn.
  pub fn submit(self: &Rc<Self>, space: &Rc<ServiceStore>, actor: Actor, call: Call, input: Value, reply: Reply) {
    self.post(space, actor, call, input, None, vec![], reply)
  }

  // Take a call that the ops of the spaces in `awaiting` wait on, under a
  // deadline that begins once the space is ready, which answering it cancels.
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn post(self: &Rc<Self>, space: &Rc<ServiceStore>, actor: Actor, call: Call, input: Value, cause: Option<String>, awaiting: Vec<String>, reply: Reply) {
    let deadline = Deadline::new(call.key(), self.timeout_of(space));
    let answering = Rc::clone(&deadline);
    let reply: Reply = Box::new(move |settled| {
      answering.answered();
      reply(settled)
    });
    self.take(space, Taken { actor, call, input, cause, awaiting, deadline, reply })
  }

  // A call taken: refused by a stopped space; a GET held while its space
  // initializes and read once it is ready; an op queued in the log. Its
  // deadline begins now, unless its space is still starting.
  pub(crate) fn take(self: &Rc<Self>, space: &Rc<ServiceStore>, taken: Taken) {
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
  fn queue(self: &Rc<Self>, space: &Rc<ServiceStore>, taken: Taken) {
    let Taken { actor, call, input, cause, awaiting, deadline, reply } = taken;
    let op = self.stamp(&actor, &call, &input, &space.runs.package);
    let (here, hash) = (Rc::clone(space), op.hash.clone());
    deadline.waiting(Box::new(move |error| here.give_up(&hash, error)));
    space.wait_for(&op.hash, awaiting, reply, deadline);
    self.log_made(space, LogRow::caused(op, cause));
  }

  // Log an op made here. When its store cannot point to it, the op was never
  // taken, so its row leaves the channel's log again, and the store stops.
  fn log_made(self: &Rc<Self>, space: &Rc<ServiceStore>, row: LogRow) {
    match point(space, &row) {
      Ok(()) => self.drain(space),
      Err(failure) => {
        let failure = oplog::remove(space.channel.log(), &row.op.hash).map_or_else(|other| other, |_| failure);
        self.stop(space, failure)
      }
    }
  }

  // The timeout a call to a space must settle within: its service's, once
  // this device has its package, compiled or compiling; the default before.
  fn timeout_of(&self, space: &ServiceStore) -> std::time::Duration {
    space.service().or_else(|| self.services().find(&space.runs.package)).map_or(services::DEFAULT_TIMEOUT, |service| service.timeout)
  }

  // Log a row another device sent, then apply what is pending. A store
  // that cannot point to it stops, and takes it up when resumed.
  pub(crate) fn log(self: &Rc<Self>, space: &Rc<ServiceStore>, row: LogRow) {
    match point(space, &row) {
      Ok(()) => self.drain(space),
      Err(error) => self.stop(space, error),
    }
  }

  // Apply a space's pending ops in turn, until one is still applying, none is
  // pending, or the space is not ready.
  pub(crate) fn drain(self: &Rc<Self>, space: &Rc<ServiceStore>) {
    if space.draining.replace(true) {
      return;
    }
    while space.run_state() == RunState::Idle {
      if let Err(failure) = self.between_turns(space) {
        self.stop(space, failure);
        break;
      }
      match next_row(space) {
        Ok(Some(row)) => self.apply(space, row),
        Ok(None) => break,
        Err(failure) => self.stop(space, failure),
      }
    }
    space.draining.set(false);
  }

  // Apply one op, with the version of its service it was made for: a first
  // run, or a replay of the effects its row carries. An op its caller gave
  // up on is refused with what the caller got, and one whose cause failed
  // with that.
  fn apply(self: &Rc<Self>, space: &Rc<ServiceStore>, row: LogRow) {
    if let Some(expired) = space.expired(&row.op.hash) {
      return self.refuse(space, &row.op, expired);
    }
    match cause_failed_for(space, &row) {
      Ok(Some(failed)) => return self.refuse(space, &row.op, failed),
      Ok(None) => {}
      Err(failure) => return self.stop(space, failure),
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
  fn begin(self: &Rc<Self>, space: &Rc<ServiceStore>, op: Op, call: Call, recorded: Option<Vec<Made>>) -> Option<Rc<ServiceCall>> {
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
  // place rolls back and rewinds the store; any other ends its transaction
  // with its result and answers whoever waits on it.
  fn finish(self: &Rc<Self>, space: &Rc<ServiceStore>, op: &Op, later: &[Recorded], result: Result<Value>, calls: Vec<CallRecord>, made: Option<Vec<Made>>) {
    let ran = Ran::of(space.store(), result.is_ok());
    let ran = match ran {
      Ok(ran) => ran,
      Err(failure) => return self.stop(space, failure),
    };
    if !rewind::fits_in_place(&ran.sets, later) {
      return self.rewind(space, op, made, later);
    }
    let before = match oplog::outcome(space.channel.log(), &op.hash) {
      Ok(before) => before,
      Err(failure) => return self.stop(space, failure),
    };
    match end(space, op, &ran, result, calls, made.as_deref()).and_then(|settled| self.unmake_refused(&settled, made.as_deref()).map(|_| settled)) {
      Ok(settled) => {
        self.set_run_state(space, RunState::Idle);
        self.outcome_changed(space, &op.hash, before, settled.is_ok());
        let settled = settled.map(|settled| Settled { watches: self.committed(&space.id, &settled.tables), ..settled });
        space.answer(&op.hash, settled);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // A refused op's first run leaves nothing behind: what it created is deleted.
  fn unmake_refused(self: &Rc<Self>, settled: &Result<Settled>, made: Option<&[Made]>) -> Result<()> {
    match (settled, made) {
      (Err(_), Some(made)) => self.unmake(made),
      _ => Ok(()),
    }
  }

  // What became of an op changed: the ops its calls made follow it, each
  // taken up again in its store (`reconsider`). Calls apply while their
  // cause is still running, as though it applied, so an op settling applied
  // for the first time changes nothing for them.
  fn outcome_changed(self: &Rc<Self>, space: &ServiceStore, hash: &str, before: Option<bool>, applied: bool) {
    if before.unwrap_or(true) == applied {
      return;
    }
    let calls = match oplog::calls(space.channel.log(), hash) {
      Ok(calls) => calls,
      Err(failure) => return eprintln!("[carbondb] the calls of {hash} are not taken up again: {}", failure.message),
    };
    for (call, service) in calls {
      if let Ok(store) = self.named_store(&Rc::clone(&space.channel), &service) {
        store.reconsider(&call);
        self.drain(&store);
      }
    }
  }

  // Take up an op again whose cause's outcome changed: when it has run here,
  // undo it and every later op that touched this store's tables, newest
  // first, forgetting their results and outcomes, so they are pending again
  // and re-run in order; at its turn it is refused if its cause has failed.
  fn reconsider_now(self: &Rc<Self>, space: &ServiceStore, hash: &str) -> Result<()> {
    let Some(pointer) = pointers::get(space.store(), hash)? else { return Ok(()) };
    let run = rewind::from(space.store(), &pointer)?;
    if run.first().is_none_or(|first| first.hash != hash) {
      return Ok(());
    }
    let undone = rewind::undo_from(&run);
    let hashes: Vec<String> = undone.iter().map(|op| op.hash.clone()).collect();
    let (_, changeset) = space.store().write(|store| {
      undone.iter().try_for_each(|op| store.undo(&op.changeset))?;
      results::forget(store, &hashes)
    })?;
    oplog::forget_outcomes(space.channel.log(), &hashes)?;
    self.committed(&space.id, &tables(&changeset)?);
    Ok(())
  }

  // Roll back a late op's run, then undo the later ops that touched the
  // space's tables, and forget their results, in one transaction: they are
  // pending again, so the late op applies next at its place, and they re-run
  // after it. Effects its first run made are kept on its row, so its run at
  // its place replays them rather than making them again.
  fn rewind(self: &Rc<Self>, space: &Rc<ServiceStore>, op: &Op, made: Option<Vec<Made>>, later: &[Recorded]) {
    if let Err(failure) = space.store().end_write(false) {
      return self.stop(space, failure);
    }
    self.set_run_state(space, RunState::Idle);
    let undone = rewind::undo_set(later);
    let hashes: Vec<String> = undone.iter().map(|op| op.hash.clone()).collect();
    let undo = made.iter().try_for_each(|made| oplog::record_effects(space.channel.log(), &op.hash, made)).and_then(|_| oplog::forget_outcomes(space.channel.log(), &hashes)).and_then(|_| {
      space.store().write(|store| {
        undone.iter().try_for_each(|op| store.undo(&op.changeset))?;
        results::forget(store, &hashes)
      })
    });
    match undo.and_then(|(_, changeset)| tables(&changeset)) {
      Ok(tables) => {
        self.committed(&space.id, &tables);
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // Refuse an op before it applies, keeping its result and its outcome.
  fn refuse(self: &Rc<Self>, space: &ServiceStore, op: &Op, error: ServiceError) {
    let recorded = oplog::record_outcome(space.channel.log(), &op.hash, false).and_then(|before| results::record(space.store(), &op.hash, &Outcome::Refused { error: error.clone(), calls: json!([]) }, &[], &[]).map(|_| before));
    match recorded {
      Ok(before) => {
        self.outcome_changed(space, &op.hash, before, false);
        space.answer(&op.hash, Err(error))
      }
      Err(failure) => self.stop(space, failure),
    }
  }

  // Stop a space after a system failure: hold nothing more for the log, and
  // answer everyone waiting on its ops with the failure.
  pub(crate) fn stop(self: &Rc<Self>, space: &ServiceStore, failure: ServiceError) {
    space.op_writer.discard();
    self.set_run_state(space, RunState::Stopped { error: failure.clone() });
    space.answer_all(&stopped(&space.called(), &failure));
    self.release(&space.id);
  }

  // Resume a stopped space once its failure is fixed: one never set up
  // initializes again; any other applies its pending ops again from the
  // first, the op that was applying when it stopped included. Returns the
  // watches it ran again.
  pub fn resume(self: &Rc<Self>, space: &Rc<ServiceStore>) -> Vec<WatchId> {
    if !matches!(space.run_state(), RunState::Stopped { .. }) {
      return vec![];
    }
    space.store().clear_failure();
    if space.service().is_none() {
      return self.initialize(space);
    }
    if let Err(failure) = take_up(space) {
      self.stop(space, failure);
      return vec![];
    }
    let rerun = self.set_run_state(space, RunState::Idle);
    self.drain(space);
    rerun
  }

  // An op applies only with the version of the service it was made for.
  fn same_version(&self, space: &ServiceStore, call: &Call, op: &Op) -> Result<()> {
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
  pub(crate) fn read(self: &Rc<Self>, space: &Rc<ServiceStore>, taken: Taken) {
    let Taken { actor, call, input, awaiting, deadline, reply, .. } = taken;
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

  fn answer_read(self: &Rc<Self>, space: &ServiceStore, reply: Reply, settled: Result<Settled>) {
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
// goes by. A local space runs `[@carbon]` only.
fn answering(carbon: &Carbon, space: &ServiceStore, call: &Call) -> Result<Rc<Service>> {
  match (space.answers(call), space.service()) {
    (true, Some(service)) => Ok(service),
    (true, None) => Err(services::missing(&space.runs.package)),
    (false, _) if space.id == carbon.system.id => Err(not_found(format!("The system answers only [@carbon], not {}", call.key()))),
    (false, _) => Err(not_found(format!("{} is not this space's service, [{}]", call.key(), space.runs.name))),
  }
}

// The `call` effect: a call to another service, posted to its store as the
// caller's actor, or, for `[@carbon]`, as the actor it names. The store is
// the one named, or the one the caller's channel names for the call's
// service. A write within the caller's channel is bound to the op making it,
// its cause; one into another channel is a message no cause binds. A write
// into a store whose op waits on this call, directly or through other calls,
// would wait behind that op until the timeout, so it is refused at once.
pub(crate) fn call_effect(caller: Rc<ServiceCall>, input: Value, reply: crate::Reply) {
  let carbon = Rc::clone(caller.carbon());
  let made = Call::from_key(input["key"].as_str().unwrap_or_default()).and_then(|call| {
    let actor = match input.get("as").filter(|_| caller.call.confined) {
      Some(actor) => serde_json::from_value(actor.clone())?,
      None => caller.actor.clone(),
    };
    let space = match input["space"].as_str() {
      Some(space) => space.to_string(),
      None => caller.space().channel.id.clone(),
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
    Ok(target) => {
      let cause = caller.cause_of(&target);
      carbon.post(&target, actor, call, input["input"].clone(), cause, awaiting, Box::new(move |settled| reply(settled.map(|settled| settled.value))))
    }
    Err(error) => reply(Err(error)),
  }
}

// A write posted back into a service whose op waits on the call posting it.
fn looped(call: &Call) -> ServiceError {
  ServiceError::new(508, "LOOP_DETECTED", format!("{} writes to a service whose op is waiting on this call, which would wait for itself", call.key()))
}

// Calls to a stopped space fail with the failure that stopped it, and so do rows received for it.
pub(crate) fn accepting(space: &ServiceStore) -> Result<()> {
  match space.run_state() {
    RunState::Stopped { error } => Err(stopped(&space.called(), &error)),
    _ => Ok(()),
  }
}

// Between two ops: write the pointers that arrived while the last applied,
// and take up again the ops whose cause's outcome changed meanwhile.
impl Carbon {
  fn between_turns(self: &Rc<Self>, space: &Rc<ServiceStore>) -> Result<()> {
    space.op_writer.write_held(space.store())?;
    space.reconsidering().iter().try_for_each(|hash| self.reconsider_now(space, hash))?;
    space.store().failure().map_or(Ok(()), Err)
  }
}

// Write a row to its channel's log, and the store's pointer to it. A row
// already logged is a duplicate.
fn point(space: &ServiceStore, row: &LogRow) -> Result<()> {
  match oplog::append(space.channel.log(), row)? {
    true => space.op_writer.write(space.store(), space.is_applying(), pointer_to(row)),
    false => Ok(()),
  }
}

// Point to every op the channel's log addresses to a store, as it opens or resumes.
pub(crate) fn take_up(space: &ServiceStore) -> Result<()> {
  let addressed = oplog::pointers(space.channel.log(), &space.runs.name)?;
  addressed.into_iter().try_for_each(|pointer| space.op_writer.write(space.store(), false, pointer))
}

// The next pending op's row, read from the channel's log.
fn next_row(space: &ServiceStore) -> Result<Option<LogRow>> {
  let Some(hash) = pointers::next_pending(space.store())? else { return Ok(None) };
  oplog::row(space.channel.log(), &hash)?.map(Some).ok_or_else(|| crate::error::internal(format!("{} points to op {hash}, which its channel's log lacks", space.called())))
}

// Why an op is refused before it runs because its cause failed here, if it did.
fn cause_failed_for(space: &ServiceStore, row: &LogRow) -> Result<Option<ServiceError>> {
  let Some(cause) = &row.cause else { return Ok(None) };
  Ok((oplog::outcome(space.channel.log(), cause)? == Some(false)).then(|| cause_failed(cause)))
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
fn end(space: &ServiceStore, op: &Op, ran: &Ran, result: Result<Value>, calls: Vec<CallRecord>, made: Option<&[Made]>) -> Result<Result<Settled>> {
  let store = space.store();
  // Rolling back while stopping: the store stops with the first failure,
  // whatever the rollback meets.
  if let Some(failure) = store.failure() {
    let _ = store.end_write(false);
    return Err(failure);
  }
  // Its effects and outcome go into the channel's log before its transaction
  // ends: a crash in between leaves the op pending, to run again replaying them.
  let logged = made.iter().try_for_each(|made| oplog::record_effects(space.channel.log(), &op.hash, made)).and_then(|_| oplog::record_outcome(space.channel.log(), &op.hash, result.is_ok()));
  if let Err(failure) = logged {
    let _ = store.end_write(false);
    return Err(failure);
  }
  match result {
    Ok(value) => {
      let applied = Outcome::Applied { output: value.clone(), calls: json!(calls) };
      if let Err(failure) = results::record(store, &op.hash, &applied, &ran.read_set, &ran.changeset) {
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
