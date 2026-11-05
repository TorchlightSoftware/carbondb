//! What a service answers: one invocation per call. A service is a package,
//! its name, migrations and entry point; the host routes each call to the entry
//! point by service name, and the entry point routes method and resource itself
//! and settles the invocation once, now or later.
//!
//! A service reaches storage only through its invocation's queries, and other
//! services only through `send`. It never sees the host.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;

use crate::call::{Actor, Call};
use crate::error::{Result, internal};
use crate::op::Op;

// What a handler's queries run against: inside an op's transaction, or a GET's
// snapshot of committed state. Only a write may `run`.
pub trait Db {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64>;
  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>>;

  // The first row, or null when there is none.
  fn get(&self, sql: &str, params: &[Value]) -> Result<Value> {
    Ok(self.all(sql, params)?.into_iter().next().unwrap_or(Value::Null))
  }
}

// How a call answers whoever made it, once.
pub type Reply = Box<dyn FnOnce(Result<Value>)>;

pub trait Invocation: Db {
  // What was called, with what input, by whom.
  fn call(&self) -> &Call;
  fn input(&self) -> &Value;
  fn actor(&self) -> &Actor;

  // The op the call writes within; none for a GET.
  fn op(&self) -> Option<&Op>;

  // The next value drawn from the op, the source of its ids and randomness.
  fn draw(&self) -> Result<String>;

  // Make a call from inside this one, in the same space and as the same
  // identity. It runs inside this call's op, sees its writes, and when it fails
  // only its own writes roll back.
  fn send(self: Rc<Self>, call: Call, input: Value, reply: Reply);

  // Settle the call, once.
  fn settle(&self, result: Result<Value>) -> Result<()>;
}

// An entry point: it receives each call to its service.
pub type Entry = Rc<dyn Fn(Rc<dyn Invocation>)>;

// A service written in Rust, as its crate exports it: its name and migrations
// from its service.toml, and its entry point.
pub struct Package {
  pub name: &'static str,
  pub migrations: &'static [&'static str],
  pub entry: fn(Rc<dyn Invocation>),
}

// Settle an invocation with what a handler returns at once.
pub fn answer(invocation: Rc<dyn Invocation>, handle: fn(&Rc<dyn Invocation>) -> Result<Value>) {
  let result = handle(&invocation);
  let _ = invocation.settle(result);
}

// Start something that answers through a callback, and return its answer,
// which must come at once: as CarbonDB's own services answer, and as a
// service reads another of them.
pub fn now<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Result<T> {
  let answer = Rc::new(RefCell::new(None));
  let slot = Rc::clone(&answer);
  start(Box::new(move |result| *slot.borrow_mut() = Some(result)));
  answer.borrow_mut().take().unwrap_or_else(|| Err(internal("A call that must answer at once answered later")))
}

// A call from inside an invocation, answered at once.
pub fn ask(invocation: &Rc<dyn Invocation>, key: &str, input: Value) -> Result<Value> {
  let call = Call::from_key(key)?;
  now(|reply| Rc::clone(invocation).send(call, input, reply))
}
