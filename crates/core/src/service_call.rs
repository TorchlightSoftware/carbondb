//! A service call: one call running in the core, which its service's entry
//! point sees as its handler context. It holds the call, its input and the
//! actor; the connection its queries run on, inside the op's transaction or a
//! GET's snapshot; the op it runs within, if it writes; the calls it has made;
//! and the way to settle it, once.
//!
//! A call made inside another runs inside the same op, under a savepoint, so
//! it sees the writes before it, and a failure rolls back only its own writes
//! and throws to the handler that made it. The one exception is a local
//! service writing to a replicated one: that is an op of its own, logged with
//! its result, since the local service never replicates.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::dispatch::Settled;
use crate::error::{Result, ServiceError, bad_request, conflict, forbidden, internal};
use crate::results::Outcome;
use crate::space::Space;
use crate::call_window::{CallWindow, Scoped};
use crate::store::Reader;
use crate::{Actor, Call, Db, HandlerContext, Op, Reply, oplog, results};

// How a service call ends: what it settled with, and the calls it made.
pub(crate) type Settle = Box<dyn FnOnce(Result<Value>, Vec<CallRecord>)>;

// The connection a service call queries: the space's writer inside an op, or a GET's own reader.
#[derive(Clone)]
pub(crate) enum Conn {
  Writer,
  Reader(Rc<Reader>),
}

// A call a service call made, kept with the op's result on this device: what
// it was asked, what it answered, and the calls it made in turn.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallRecord {
  pub key: String,
  pub input: Value,
  // The space it ran in, when not the caller's.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub space: Option<String>,
  // Its own op's hash, when it ran as an op of its own.
  #[serde(skip_serializing_if = "Option::is_none")]
  pub op: Option<String>,
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
    CallRecord { key: call.key(), input, space: None, op: None, output, error, calls }
  }
}

// How a call made inside another runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Within {
  // Inside the caller's op, re-run with it on every replica.
  SameOp,
  // As an op of its own: a local service writing to a replicated one.
  OwnOp,
}

// Who may call what, and how: a replicated service never calls a local one,
// since every replica would repeat its effects; a GET never calls a write.
fn within(caller: &Call, callee: &Call) -> Result<Within> {
  match (caller.confined, caller.writes(), callee.confined, callee.writes()) {
    (false, _, true, _) => Err(forbidden(format!("A replicated service never calls a local one, such as {}", callee.key()))),
    (_, false, _, true) => Err(bad_request(format!("A GET cannot call {}, which writes", callee.key()))),
    (true, _, false, true) => Ok(Within::OwnOp),
    _ => Ok(Within::SameOp),
  }
}

pub struct ServiceCall {
  carbon: Rc<Carbon>,
  space: Rc<Space>,
  pub actor: Actor,
  pub call: Call,
  pub input: Value,
  conn: Conn,
  // The op it runs within, when it writes. Calls within one op share it, and so its draws.
  op: Option<Op>,
  // Whether the top-level call it belongs to is still open; nothing queries after.
  live: Rc<Cell<bool>>,
  calls: RefCell<Vec<CallRecord>>,
  pending: Cell<usize>,
  settle: RefCell<Option<Settle>>,
}

impl ServiceCall {
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn new(
    carbon: &Rc<Carbon>,
    space: &Rc<Space>,
    actor: Actor,
    call: Call,
    input: Value,
    conn: Conn,
    op: Option<Op>,
    live: Rc<Cell<bool>>,
    settle: Settle,
  ) -> Rc<ServiceCall> {
    Rc::new(ServiceCall {
      carbon: Rc::clone(carbon),
      space: Rc::clone(space),
      actor,
      call,
      input,
      conn,
      op,
      live,
      calls: RefCell::default(),
      pending: Cell::new(0),
      settle: RefCell::new(Some(settle)),
    })
  }

  pub fn carbon(&self) -> &Rc<Carbon> {
    &self.carbon
  }

  pub fn space(&self) -> &Rc<Space> {
    &self.space
  }

  // The op being applied, when this call writes within it.
  fn writing(&self) -> Option<&Op> {
    self.op.as_ref().filter(|_| self.call.writes())
  }

  // Make a call in a space, as an actor. Only a local service calls into
  // another space, or as another actor, as when it sets up a new identity.
  pub fn call_in(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    if !self.live.get() {
      return reply(Err(self.ended()));
    }
    if !Rc::ptr_eq(space, &self.space) || actor != self.actor {
      return self.elsewhere(space, actor, call, input, reply);
    }
    match within(&self.call, &call) {
      Ok(within) => self.inside(actor, call, input, within, reply),
      Err(error) => reply(Err(error)),
    }
  }

  // A local service's call into another space, or as another actor: an op of
  // that space's own, applied when its turn there comes.
  fn elsewhere(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    if !self.call.confined {
      return reply(Err(forbidden(format!("Only a local service calls into another space or as another actor, not {}", self.call.key()))));
    }
    if Rc::ptr_eq(space, &self.space) && call.writes() {
      return self.inside(actor, call, input, Within::OwnOp, reply);
    }
    self.pending.set(self.pending.get() + 1);
    let caller = Rc::clone(self);
    let (record_call, record_input, id) = (call.clone(), input.clone(), space.id.clone());
    self.carbon.submit(space, actor, call, input, Box::new(move |settled: Result<Settled>| {
      let result = settled.map(|settled| settled.value);
      let mut record = CallRecord::of(&record_call, record_input, &result, vec![]);
      record.space = Some(id);
      caller.returned(record);
      reply(result)
    }));
  }

  // A call inside this one's transaction: under a savepoint when it writes, in
  // the same op or an op of its own.
  fn inside(self: &Rc<Self>, actor: Actor, call: Call, input: Value, within: Within, reply: Reply) {
    let guarded = self.call.writes();
    if guarded && let Err(error) = self.space.store().savepoint() {
      return reply(Err(error));
    }
    let own_op = (within == Within::OwnOp).then(|| self.carbon.stamp(&actor, &call, &input));
    let op = own_op.clone().or_else(|| self.op.clone());
    self.pending.set(self.pending.get() + 1);
    let caller = Rc::clone(self);
    let (record_call, record_input) = (call.clone(), input.clone());
    let settle: Settle = Box::new(move |result, calls| {
      let result = if guarded { caller.close(result, own_op.as_ref(), &calls) } else { result };
      let mut record = CallRecord::of(&record_call, record_input, &result, calls);
      record.op = own_op.map(|op| op.hash);
      caller.returned(record);
      reply(result)
    });
    let callee = ServiceCall::new(&self.carbon, &self.space, actor, call, input, self.conn.clone(), op, Rc::clone(&self.live), settle);
    self.carbon.route(callee);
  }

  // End a call's savepoint, keeping its writes or rolling them back. An op of
  // its own goes in the log with its result either way.
  fn close(&self, result: Result<Value>, own_op: Option<&Op>, calls: &[CallRecord]) -> Result<Value> {
    if !self.live.get() {
      return Err(self.ended());
    }
    let store = self.space.store();
    let closed = store.end_savepoint(result.is_ok());
    let logged = own_op.map_or(Ok(()), |op| oplog::append(store, op).and_then(|_| results::record(store, &op.hash, &Outcome::of(&result, json!(calls)))));
    closed.and(logged).and(result)
  }

  // A call this one made has answered.
  fn returned(&self, record: CallRecord) {
    self.pending.set(self.pending.get() - 1);
    self.calls.borrow_mut().push(record);
  }

  fn ended(&self) -> ServiceError {
    internal(format!("{} has ended; nothing more runs inside it", self.call.key()))
  }

  // The `db` binding: the service's namespace on this call's connection, with its op.
  fn db(&self) -> Result<Scoped<'_>> {
    if !self.live.get() {
      return Err(self.ended());
    }
    let prefix = self.carbon.services().get(&self.call.service)?.prefix()?;
    let window = CallWindow { prefix, op: self.writing().cloned() };
    Ok(match &self.conn {
      Conn::Writer => self.space.store().scoped(window),
      Conn::Reader(reader) => reader.scoped(window),
    })
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
    match Call::from_key(key) {
      Ok(call) => {
        let (space, actor) = (Rc::clone(&self.space), self.actor.clone());
        self.call_in(&space, actor, call, input, reply)
      }
      Err(error) => reply(Err(error)),
    }
  }

  // A handler that finishes before the calls it made fails, since their writes
  // would land outside its op.
  fn settle(&self, result: Result<Value>) -> Result<()> {
    let settle = self.settle.borrow_mut().take().ok_or_else(|| conflict(format!("{} has already settled", self.call.key())))?;
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
  fn decides_how_a_call_inside_another_runs() {
    let key = |key: &str| Call::from_key(key).unwrap();
    let cases = [
      ("POST [chat]/messages", "POST [space]/ref", Ok(Within::SameOp)),
      ("POST [chat]/messages", "GET [space]/refs", Ok(Within::SameOp)),
      ("GET [chat]/messages", "GET [space]/refs", Ok(Within::SameOp)),
      ("POST [@carbon]/space", "POST [space]/ref", Ok(Within::OwnOp)),
      ("POST [@carbon]/space", "GET [space]/refs", Ok(Within::SameOp)),
      ("POST [@carbon]/space", "GET [@carbon]/status", Ok(Within::SameOp)),
      ("POST [chat]/messages", "GET [@carbon]/status", Err(403)),
      ("GET [chat]/messages", "POST [space]/ref", Err(400)),
      ("GET [@carbon]/status", "POST [space]/ref", Err(400)),
    ];
    for (caller, callee, expected) in cases {
      assert_eq!(within(&key(caller), &key(callee)).map_err(|error| error.status), expected, "{caller} -> {callee}");
    }
  }
}
