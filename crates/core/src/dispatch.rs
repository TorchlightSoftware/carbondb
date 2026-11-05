//! One path for every call, whoever makes it and whichever service answers it.
//!
//! A call submitted to a space either reads or writes. A GET reads at once, on
//! its own snapshot of committed state, and never waits. Anything else is an op:
//! it waits its turn in the space's RunQueue, is stamped when its turn comes so
//! it follows everything applied before it, and applies in one transaction,
//! appended to the op log when it succeeds. Either way the call is routed by
//! service name to the service's entry point, which settles it.

use std::cell::Cell;
use std::rc::Rc;

use serde::Serialize;
use serde_json::{Value, json};

use crate::carbon::Carbon;
use crate::error::{Result, not_found};
use crate::frame::{Conn, Frame, Settle};
use crate::op::Applying;
use crate::oplog;
use crate::services::Service;
use crate::space::Space;
use crate::{Actor, Call, Invocation, Op};

// What a call settled with, and the tables it read or wrote, for watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settled {
  pub value: Value,
  pub tables: Vec<String>,
}

// How a submitted call answers whoever submitted it.
pub type Reply = Box<dyn FnOnce(Result<Settled>)>;

// An op waiting its turn in a space's RunQueue.
pub struct Job {
  actor: Actor,
  call: Call,
  input: Value,
  reply: Reply,
}

impl Carbon {
  // Take a call on a space: a GET reads now, anything else waits its turn.
  pub fn submit(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    if !call.writes() {
      return self.read(space, actor, call, input, reply);
    }
    space.queue.borrow_mut().push(Job { actor, call, input, reply });
    self.drain(space);
  }

  // Apply the space's waiting ops in turn, until one is still running or none wait.
  fn drain(self: &Rc<Self>, space: &Rc<Space>) {
    if !space.queue.borrow_mut().begin_drain() {
      return;
    }
    loop {
      let next = space.queue.borrow_mut().take();
      let Some(job) = next else { break };
      self.apply(space, job);
      if space.queue.borrow().is_running() {
        break;
      }
    }
    space.queue.borrow_mut().end_drain();
  }

  // Apply one op: stamp it, open its transaction and route it. When it
  // settles, commit it with its log entry or roll it back, and hand the turn on.
  fn apply(self: &Rc<Self>, space: &Rc<Space>, Job { actor, call, input, reply }: Job) {
    let applying = Rc::new(Applying::new(self.stamp(&actor, &call, &input)));
    let live = Rc::new(Cell::new(true));
    let (carbon, here, op, ended) = (Rc::clone(self), Rc::clone(space), Rc::clone(&applying), Rc::clone(&live));
    let settle: Settle = Box::new(move |result, calls| {
      ended.set(false);
      let settled = commit(&here, &op.op, result, &json!(calls));
      here.queue.borrow_mut().finish();
      if let Ok(settled) = &settled {
        here.committed(&settled.tables);
      }
      reply(settled);
      carbon.drain(&here);
    });
    let frame = Frame::new(self, space, actor, call, input, Conn::Writer, Some(applying), live, settle);
    match space.store().begin_write() {
      Ok(()) => self.route(frame),
      Err(error) => {
        let _ = frame.settle(Err(error));
      }
    }
  }

  // Run a GET on its own snapshot of committed state.
  fn read(self: &Rc<Self>, space: &Rc<Space>, actor: Actor, call: Call, input: Value, reply: Reply) {
    let reader = match space.store().reader() {
      Ok(reader) => Rc::new(reader),
      Err(error) => return reply(Err(error)),
    };
    let live = Rc::new(Cell::new(true));
    let (snapshot, ended) = (Rc::clone(&reader), Rc::clone(&live));
    let settle: Settle = Box::new(move |result, _calls| {
      ended.set(false);
      let tables = snapshot.end();
      reply(result.and_then(|value| Ok(Settled { value, tables: tables? })))
    });
    self.route(Frame::new(self, space, actor, call, input, Conn::Reader(reader), None, live, settle));
  }

  // Hand a call to the entry point of the service that answers it here.
  pub(crate) fn route(&self, frame: Rc<Frame>) {
    match answering(self, frame.space(), &frame.call) {
      Ok(service) => (service.entry)(frame),
      Err(error) => {
        let _ = frame.settle(Err(error));
      }
    }
  }
}

// The service a call reaches, by name: a local service this device runs, or a
// replicated one this space runs. The root space runs local services only.
fn answering(carbon: &Carbon, space: &Space, call: &Call) -> Result<Rc<Service>> {
  let found = carbon.services().find(&call.service).filter(|service| service.local == call.confined);
  match (call.confined, found) {
    (true, Some(service)) => Ok(service),
    (true, None) => Err(not_found(format!("No local service [@{}] on this device", call.service))),
    (false, _) if space.is_root() => Err(not_found(format!("The system handle answers only local services, not {}", call.key()))),
    (false, Some(service)) if space.runs(&call.service) => Ok(service),
    (false, _) => Err(not_found(format!("[{}] does not run in this space", call.service))),
  }
}

// End an op's transaction: commit it with its log entry when it succeeded, or
// roll it back, in which case it is never logged.
fn commit(space: &Space, op: &Op, result: Result<Value>, calls: &Value) -> Result<Settled> {
  let store = space.store();
  let result = result.and_then(|value| oplog::append(store, op, &value, calls).map(|_| value));
  let tables = store.end_write(result.is_ok());
  let value = result?;
  Ok(Settled { value, tables: tables? })
}
