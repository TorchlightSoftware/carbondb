//! The handler context: what a service sees of each call made to it.

use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::op::Op;

// The `db` binding: the service's own tables, inside an op's transaction or a
// GET's snapshot of committed state. Only a write may `run`. A `run` with no
// parameters may hold several statements, as a migration does.
pub trait Db {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64>;
  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>>;

  // The first row, or null when there is none.
  fn get(&self, sql: &str, params: &[Value]) -> Result<Value> {
    Ok(self.all(sql, params)?.into_iter().next().unwrap_or(Value::Null))
  }
}

// Who is acting: the identity making the call, and the device it acts from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
  pub identity: Option<String>,
  pub device: Option<String>,
}

// How a call answers whoever made it, once.
pub type Reply = Box<dyn FnOnce(Result<Value>)>;

pub trait HandlerContext: Db {
  // The service call: what was asked of this service, with what input, by whom.
  fn method(&self) -> &str;
  fn resource(&self) -> &str;
  fn input(&self) -> &Value;
  fn actor(&self) -> &Actor;

  // The op the call writes within, with its ids and randomness; none for a GET.
  fn op(&self) -> Option<&Op>;

  // Make a call by its service key, in the same space and as the same actor.
  // Inside an op it runs within the op, sees its writes, and when it fails only
  // its own writes roll back. It answers through `reply`, now or later.
  fn call(self: Rc<Self>, key: &str, input: Value, reply: Reply);

  // Settle the call, once.
  fn settle(&self, result: Result<Value>) -> Result<()>;
}

// A Rust service as its crate exports it: its name, version and `sql` binding
// from its service.toml, the `init` that sets up its schema on every open, and
// its entry point.
pub struct Package {
  pub name: &'static str,
  pub version: &'static str,
  pub binding: Option<&'static str>,
  pub init: fn(&dyn Db) -> Result<()>,
  pub entry: fn(Rc<dyn HandlerContext>),
}
