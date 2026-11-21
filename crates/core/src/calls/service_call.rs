//! A service call: one call running in the core, which its service's entry
//! point sees as its handler context. It holds the call, its input and the
//! actor; the connection its queries run on, inside the op's transaction or a
//! GET's snapshot; the op it runs within, if it writes; the op's effects; the
//! calls it has made; and the way to settle it, once.
//!
//! A call to the same service runs inside the same op, under a savepoint, so
//! it sees the writes before it, and a failure rolls back only its own writes
//! and throws to the handler that made it, whichever binding makes it.
//!
//! A call to another service in the channel takes one of two bindings:
//!
//! - **`call`**, a message made as the `call` effect: on the op's first run it
//!   is posted to that service and its answer recorded; every later run gets
//!   the recorded answer (`ops::effects`). A write it posts is bound to no
//!   cause, so it stays when its caller is later refused.
//! - **`callsync`**, consistent with the caller's place in the shared order. A
//!   write is the same message, bound to the op making it, its cause, which it
//!   follows. A read runs on every run, once the service read has applied
//!   every op before the caller's place, each row read as it stood then, and
//!   is kept in the service read as read by the caller's op (`ops::reads`), so
//!   an op applied there at an earlier place re-runs it.
//!
//! A call that outlives its deadline expires: the core settles it with a
//! timeout, and its handler's own answer, when it comes, is dropped.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use serde_json::json;

use crate::carbon::{Cancel, Carbon};
use crate::calls::deadline::Deadline;
use crate::error::{Result, ServiceError, bad_request, conflict, forbidden, internal, timed_out};
use crate::ops::effects::{EffectLog, Turn};
use crate::ops::reads::Reading;
use crate::channels::service::ServiceStore;
use crate::sql::call_window::CallWindow;
use crate::store::Reader;
use crate::store::as_of::AsOf;
use crate::{Actor, Call, Routing, Db, HandlerContext, Op, Reply};

// How a service call ends: what it settled with, and the calls it made.
pub(crate) type Settle = Box<dyn FnOnce(Result<Value>, Vec<CallRecord>)>;

// The connection a service call queries: the space's writer inside an op, a
// GET's own reader, or, for a read through `callsync` made within an op, a
// read as of that op's place.
#[derive(Clone)]
pub(crate) enum Conn {
  Writer,
  Reader(Rc<Reader>),
  AsOf(Rc<AsOf>),
}

// A call a service call made, kept with the op's result on this device: what
// it was asked, what it answered, and the calls it made in turn.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallRecord {
  pub key: String,
  pub input: Value,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub output: Option<Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<ServiceError>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub calls: Vec<CallRecord>,
}

impl CallRecord {
  fn of(call: &Call, input: Value, result: &Result<Value>, calls: Vec<CallRecord>) -> CallRecord {
    let (output, error) = match result {
      Ok(value) => (Some(value.clone()), None),
      Err(error) => (None, Some(error.clone())),
    };
    CallRecord { key: call.key(), input, output, error, calls }
  }
}

// Who may call what: a GET never calls a write.
fn allowed(caller: &Call, callee: &Call) -> Result<()> {
  match caller.writes() || !callee.writes() {
    true => Ok(()),
    false => Err(bad_request(format!("A GET cannot call {}, which writes", callee.key()))),
  }
}

// What asking for an effect gets: a turn to make it, in its slot of the
// run's log when it is recorded; or the output the op's first run recorded.
pub(crate) enum Asked {
  Make(Option<usize>),
  Recorded(Result<Value>),
}

pub struct ServiceCall {
  carbon: Rc<Carbon>,
  space: Rc<ServiceStore>,
  pub actor: Actor,
  pub call: Call,
  pub input: Value,
  conn: Conn,
  // The op it runs within, when it writes. Calls within one op share it, and so its draws.
  op: Option<Op>,
  // The effects of the run it belongs to, shared by every call within it.
  effects: Rc<RefCell<EffectLog>>,
  // The stores whose ops wait on the run it belongs to.
  awaiting: Vec<String>,
  // The op whose reads it makes, by its place, when it runs within one or is
  // a `callsync` read made inside one: what its reads of another service are
  // as of, and who they are kept as read by.
  reading: Option<Reading>,
  // Whether the top-level call it belongs to is still open; nothing queries after.
  live: Rc<Cell<bool>>,
  calls: RefCell<Vec<CallRecord>>,
  pending: Cell<usize>,
  settle: RefCell<Option<Settle>>,
  // Its deadline, when it is a top-level call.
  deadline: RefCell<Option<Rc<Deadline>>>,
  // Whether the core settled it when its deadline passed.
  expired: Cell<bool>,
  // Drops whatever runs its handler, when it expires first.
  abandon: RefCell<Option<Cancel>>,
}

impl ServiceCall {
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn new(
    carbon: &Rc<Carbon>,
    space: &Rc<ServiceStore>,
    actor: Actor,
    call: Call,
    input: Value,
    conn: Conn,
    op: Option<Op>,
    effects: Rc<RefCell<EffectLog>>,
    awaiting: Vec<String>,
    live: Rc<Cell<bool>>,
    settle: Settle,
    reading: Option<Reading>,
  ) -> Rc<ServiceCall> {
    Rc::new(ServiceCall {
      carbon: Rc::clone(carbon),
      space: Rc::clone(space),
      actor,
      call,
      input,
      conn,
      op,
      effects,
      awaiting,
      reading,
      live,
      calls: RefCell::default(),
      pending: Cell::new(0),
      settle: RefCell::new(Some(settle)),
      deadline: RefCell::default(),
      expired: Cell::new(false),
      abandon: RefCell::default(),
    })
  }

  // Ask for an effect, as a handler does, in the order it asks: an effect
  // made on every run takes no slot; any other takes the next slot of the
  // run's log, or gets what the op's first run recorded. Asking and making
  // are apart so an effect whose input arrives later, as an outgoing
  // request's body does, still takes its slot when it is asked for.
  pub(crate) fn ask_effect(&self, name: &str, input: &Value) -> Result<Asked> {
    if !self.live.get() {
      return Err(self.ended());
    }
    let always = self.space.service().is_some_and(|service| service.always.iter().any(|effect| effect == name));
    if always {
      return Ok(Asked::Make(None));
    }
    let turn = self.effects.borrow_mut().next(name, input);
    Ok(match turn {
      Turn::Make(slot) => Asked::Make(Some(slot)),
      Turn::Recorded(output) => Asked::Recorded(output),
    })
  }

  // Make an effect asked for, keeping its output in its slot, if it took one.
  pub(crate) fn make_effect(self: Rc<Self>, slot: Option<usize>, name: &str, input: Value, reply: Reply) {
    self.pending.set(self.pending.get() + 1);
    let (caller, made_with) = (Rc::clone(&self), input.clone());
    let made: Reply = Box::new(move |output| {
      caller.pending.set(caller.pending.get() - 1);
      if let Some(slot) = slot {
        caller.effects.borrow_mut().keep(slot, made_with, output.clone());
      }
      reply(output)
    });
    crate::ops::effects::make(self, name, input, made)
  }

  // How to drop whatever runs its handler, should it expire first.
  pub(crate) fn abandon_with(&self, cancel: Cancel) {
    self.abandon.replace(Some(cancel));
  }

  // The deadline it must settle by, which settling cancels.
  pub(crate) fn set_deadline(self: &Rc<Self>, deadline: Rc<Deadline>) {
    deadline.running(self);
    self.deadline.replace(Some(deadline));
  }

  // Its deadline passed: settle it with a timeout, unless it has settled.
  pub(crate) fn expire(&self, timeout: Duration) {
    self.deadline.take();
    let Some(settle) = self.settle.borrow_mut().take() else { return };
    self.expired.set(true);
    settle(Err(timed_out(&self.call.key(), timeout)), self.calls.take());
    if let Some(abandon) = self.abandon.take() {
      abandon();
    }
  }

  pub fn carbon(&self) -> &Rc<Carbon> {
    &self.carbon
  }

  pub fn space(&self) -> &Rc<ServiceStore> {
    &self.space
  }

  // The spaces whose ops wait on the run this call belongs to.
  pub(crate) fn awaiting(&self) -> &[String] {
    &self.awaiting
  }

  // Whether the run this call belongs to is still open.
  pub(crate) fn is_live(&self) -> bool {
    self.live.get()
  }

  // The cause a write this call makes into a store is bound to: the op it
  // runs within, when the store is in the same channel; none for another
  // channel's, which no cause binds.
  pub(crate) fn cause_of(&self, target: &ServiceStore) -> Option<String> {
    self.writing().filter(|_| Rc::ptr_eq(&target.channel, &self.space.channel)).map(|op| op.hash.clone())
  }

  // The op being applied, when this call writes within it.
  fn writing(&self) -> Option<&Op> {
    self.op.as_ref().filter(|_| self.call.writes())
  }

  // Make a call in another space, as an actor: the `call` effect. Only a local
  // service names the space, or acts as another, as when it founds an identity.
  pub fn call_in(self: &Rc<Self>, space: &str, actor: Actor, call: Call, input: Value, reply: Reply) {
    if !self.call.confined {
      return reply(Err(forbidden(format!("Only a local service calls into a space by its id, or as another actor, not {}", self.call.key()))));
    }
    let request = json!({"space": space, "key": call.key(), "input": input, "as": actor, "caused": true});
    HandlerContext::effect(Rc::clone(self), "call", request, reply)
  }

  // A call to this same service, inside this one's op: under a savepoint when it writes.
  fn inside(self: &Rc<Self>, call: Call, input: Value, reply: Reply) {
    let guarded = self.call.writes();
    if guarded && let Err(error) = self.space.store().savepoint() {
      return reply(Err(error));
    }
    self.pending.set(self.pending.get() + 1);
    let caller = Rc::clone(self);
    let (record_call, record_input) = (call.clone(), input.clone());
    let settle: Settle = Box::new(move |result, calls| {
      let result = if guarded { caller.close(result) } else { result };
      caller.answered(&record_call, record_input, result, calls, reply)
    });
    let callee = ServiceCall::new(&self.carbon, &self.space, self.actor.clone(), call, input, self.conn.clone(), self.op.clone(), Rc::clone(&self.effects), self.awaiting.clone(), Rc::clone(&self.live), settle, self.reading.clone());
    self.carbon.route(callee);
  }

  // A call to another service: a message, made as the `call` effect and kept
  // among the calls this one made; a write it posts is bound to this call's
  // op when `caused`.
  fn message(self: &Rc<Self>, call: Call, input: Value, caused: bool, reply: Reply) {
    let caller = Rc::clone(self);
    let (record_call, record_input) = (call.clone(), input.clone());
    let request = json!({"key": call.key(), "input": input, "caused": caused});
    HandlerContext::effect(Rc::clone(self), "call", request, Box::new(move |result| {
      caller.calls.borrow_mut().push(CallRecord::of(&record_call, record_input, &result, vec![]));
      reply(result)
    }));
  }

  // A `callsync` read of another service in this one's channel: answered once
  // that service has applied every op it holds before this call's place, and
  // read as of it. A GET outside any op has no place, and reads as things stand.
  fn read_sync(self: &Rc<Self>, call: Call, input: Value, reply: Reply) {
    let target = match self.carbon.service_space(&self.space.channel.id, &call) {
      Ok(target) => target,
      Err(error) => return reply(Err(error)),
    };
    self.pending.set(self.pending.get() + 1);
    let (caller, read) = (Rc::clone(self), Rc::clone(&target));
    self.carbon.when_caught_up(&target, self.reading.clone(), Box::new(move |caught_up| match caught_up {
      Ok(()) => caller.read_in(&read, call, input, reply),
      Err(error) => caller.answered(&call, input, Err(error), vec![], reply),
    }));
  }

  // Run a `callsync` read in the service read, on a reader as of this call's
  // place, keeping there what it read. Its effects are this call's run's.
  fn read_in(self: &Rc<Self>, target: &Rc<ServiceStore>, call: Call, input: Value, reply: Reply) {
    let opened = match &self.reading {
      Some(reading) => target.store().reader_as_of(reading.clone()).map(|as_of| Conn::AsOf(Rc::new(as_of))),
      None => target.store().reader().map(|reader| Conn::Reader(Rc::new(reader))),
    };
    let conn = match opened {
      Ok(conn) => conn,
      Err(error) => return self.answered(&call, input, Err(error), vec![], reply),
    };
    let (caller, read, here, record_call, record_input) = (Rc::clone(self), conn.clone(), Rc::clone(target), call.clone(), input.clone());
    let settle: Settle = Box::new(move |result, calls| {
      let kept = keep_reads(&here, &read);
      caller.answered(&record_call, record_input, kept.and(result), calls, reply)
    });
    let callee = ServiceCall::new(&self.carbon, target, self.actor.clone(), call, input, conn, None, Rc::clone(&self.effects), self.awaiting.clone(), Rc::clone(&self.live), settle, self.reading.clone());
    self.carbon.route(callee);
  }

  // A call this one made inside its run has answered: keep it among its calls, and answer.
  fn answered(&self, call: &Call, input: Value, result: Result<Value>, calls: Vec<CallRecord>, reply: Reply) {
    self.returned(CallRecord::of(call, input, &result, calls));
    reply(result)
  }

  // End a call's savepoint, keeping its writes or rolling them back.
  fn close(&self, result: Result<Value>) -> Result<Value> {
    if !self.live.get() {
      return Err(self.ended());
    }
    self.space.store().end_savepoint(result.is_ok()).and(result)
  }

  // A call this one made has answered.
  fn returned(&self, record: CallRecord) {
    self.pending.set(self.pending.get() - 1);
    self.calls.borrow_mut().push(record);
  }

  fn ended(&self) -> ServiceError {
    internal(format!("{} has ended; nothing more runs inside it", self.call.key()))
  }

  // The `db` binding: the service's own tables on this call's connection,
  // with its op. `[@carbon]`'s statements are the core's own.
  fn db(&self) -> Result<Box<dyn Db + '_>> {
    if !self.live.get() {
      return Err(self.ended());
    }
    let window = CallWindow { service: !self.call.confined, op: self.writing().cloned() };
    Ok(match &self.conn {
      Conn::Writer => Box::new(self.space.store().scoped(window)),
      Conn::Reader(reader) => Box::new(reader.scoped(window)),
      Conn::AsOf(as_of) => Box::new(as_of.scoped(window)),
    })
  }
}

// End a `callsync` read. One as of an op's place keeps the rows it read in
// the service read, as read by that op; one made outside an op reads only
// what is there now, which no later op changes.
fn keep_reads(target: &ServiceStore, conn: &Conn) -> Result<()> {
  match conn {
    Conn::AsOf(as_of) => target.op_writer.record_reads(target.store(), target.is_applying(), as_of.end()?, as_of.reading().clone()),
    Conn::Reader(reader) => reader.end().map(|_| ()),
    Conn::Writer => Ok(()),
  }
}

// A handler's queries, inside its op's transaction or its GET's snapshot.
impl Db for ServiceCall {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    if !self.call.writes() {
      return Err(bad_request(format!("{} is a GET, which never writes", self.call.key())));
    }
    self.db()?.run(sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    self.db()?.all(sql, params)
  }
}

impl HandlerContext for ServiceCall {
  fn method(&self) -> &str {
    self.call.method()
  }

  fn resource(&self) -> &str {
    &self.call.resource
  }

  fn input(&self) -> &Value {
    &self.input
  }

  fn actor(&self) -> &Actor {
    &self.actor
  }

  fn op(&self) -> Option<&Op> {
    self.writing()
  }

  fn call(self: Rc<Self>, key: &str, input: Value, reply: Reply) {
    if !self.live.get() {
      return reply(Err(self.ended()));
    }
    let call = match Call::from_key(key).and_then(|call| allowed(&self.call, &call).map(|_| call)) {
      Ok(call) => call,
      Err(error) => return reply(Err(error)),
    };
    match self.space.answers(&call) {
      true => self.inside(call, input, reply),
      false => self.message(call, input, false, reply),
    }
  }

  fn callsync(self: Rc<Self>, key: &str, input: Value, reply: Reply) {
    if !self.live.get() {
      return reply(Err(self.ended()));
    }
    let call = match Call::from_key(key).and_then(|call| allowed(&self.call, &call).map(|_| call)) {
      Ok(call) => call,
      Err(error) => return reply(Err(error)),
    };
    match (self.space.answers(&call), call.writes()) {
      (true, _) => self.inside(call, input, reply),
      (false, true) => self.message(call, input, true, reply),
      (false, false) => self.read_sync(call, input, reply),
    }
  }

  fn effect(self: Rc<Self>, name: &str, input: Value, reply: Reply) {
    match self.ask_effect(name, &input) {
      Ok(Asked::Make(slot)) => self.make_effect(slot, name, input, reply),
      Ok(Asked::Recorded(output)) => reply(output),
      Err(error) => reply(Err(error)),
    }
  }

  // A handler that finishes before the calls it made fails, since their writes
  // would land outside its op. Settling an expired call drops the answer.
  fn settle(&self, result: Result<Value>) -> Result<()> {
    let taken = self.settle.borrow_mut().take();
    let Some(settle) = taken else {
      return match self.expired.get() {
        true => Ok(()),
        false => Err(conflict(format!("{} has already settled", self.call.key()))),
      };
    };
    if let Some(deadline) = self.deadline.take() {
      deadline.answered();
    }
    let result = match result {
      Ok(_) if self.pending.get() > 0 => Err(internal(format!("{} finished before the calls it made", self.call.key()))),
      result => result,
    };
    settle(result, self.calls.take());
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decides_who_may_call_what() {
    let key = |key: &str| Call::from_key(key).unwrap();
    let cases = [
      ("POST [chat]/messages", "POST [tag]/tags", Ok(())),
      ("POST [chat]/messages", "GET [channel]/members", Ok(())),
      ("GET [chat]/messages", "GET [tag]/tags", Ok(())),
      ("POST [@carbon]/identity", "POST [channel]/origin", Ok(())),
      ("POST [chat]/messages", "GET [@carbon]/status", Ok(())),
      ("POST [chat]/messages", "POST [@carbon]/services", Ok(())),
      ("GET [chat]/messages", "POST [tag]/tags", Err(400)),
      ("GET [@carbon]/status", "POST [channel]/ref", Err(400)),
    ];
    for (caller, callee, expected) in cases {
      assert_eq!(allowed(&key(caller), &key(callee)).map_err(|error| error.status), expected, "{caller} -> {callee}");
    }
  }
}
