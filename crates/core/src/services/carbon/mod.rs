//! `[@carbon]`, the device's local service: host code that runs once, on this
//! device, and never replicates. It answers on the system handle, where
//! identities are created and listed, and in every space, where spaces are
//! created and the space's status is read. Calls that need peers, or that are
//! designed but not built, are refused here.
//!
//! It is part of the core, not a service like the others: it reaches keys,
//! space files and the system database directly, and later p2panda. Everything
//! else it learns or changes in a replicated space goes through that space's
//! services: reads as calls, and writes as ops there, so every replica sees
//! them. Its own call is logged where it ran, marked with this device.

mod identities;
mod spaces;
mod steps;

use std::rc::Rc;

use serde_json::{Value, json};

use crate::error::{Result, not_built, single_node};
use crate::service_call::ServiceCall;
use crate::services::Service;
use crate::{Db, HandlerContext, Routing};

pub use identities::{Identity, SYSTEM_MIGRATIONS, acting, find};

// `[@carbon]` as the core registers it. It is not a package: no peer checks
// its version, so its version is the core's own.
pub fn service() -> Service {
  Service {
    name: "carbon".into(),
    local: true,
    version: concat!("carbondb-core@", env!("CARGO_PKG_VERSION")).into(),
    binding: None,
    timeout: crate::services::DEFAULT_TIMEOUT,
    init: Rc::new(|_: Rc<dyn Db>| Ok(())),
    entry: Rc::new(entry),
  }
}

fn entry(call: Rc<ServiceCall>) {
  let root = call.space().is_root();
  let answer = |result: Result<Value>| {
    let _ = call.settle(result);
  };
  match (root, call.call.method(), call.call.resource.as_str()) {
    (true, "POST", "/identity") => identities::create(Rc::clone(&call)),
    (true, "GET", "/identities") => answer(identities::list(call.as_ref()).map(|identities| json!(identities))),
    (true, "POST", "/devices") => answer(Err(single_node("Adding a device"))),
    (false, "GET", "/status") => status(Rc::clone(&call)),
    (false, "POST", "/space") => spaces::create(Rc::clone(&call)),
    (false, "GET", "/manage") => {
      let caller = Rc::clone(&call);
      HandlerContext::call(Rc::clone(&call), "GET [space]/manage", Value::Null, Box::new(move |managers| {
        let _ = caller.settle(managers);
      }))
    }
    (false, _, "/members" | "/join" | "/sync/start" | "/sync/stop") => answer(Err(single_node(&call.call.key()))),
    (false, "POST", "/manage") => answer(Err(not_built("Changing a manage list"))),
    (false, "DELETE", "/data") => answer(Err(not_built("Deleting a space's data"))),
    _ => answer(Err(call.call.unknown())),
  }
}

// What a space is, and its state on this device. Single-node: sync is off, there
// are no peers, and every identity here has full access.
fn status(call: Rc<ServiceCall>) {
  let asking = Rc::clone(&call);
  HandlerContext::call(Rc::clone(&call), "GET [space]/origin", Value::Null, Box::new(move |origin| match origin {
    Err(error) => {
      let _ = asking.settle(Err(error));
    }
    Ok(origin) => {
      let caller = Rc::clone(&asking);
      HandlerContext::call(asking, "GET [space]/manage", Value::Null, Box::new(move |managers| {
        let _ = caller.settle(managers.map(|managers| describe(&caller, &origin, &managers)));
      }))
    }
  }))
}

fn describe(call: &ServiceCall, origin: &Value, managers: &Value) -> Value {
  let manages = call.actor.identity.as_ref().is_some_and(|identity| managers.as_array().is_some_and(|managers| managers.contains(&json!(identity))));
  json!({
    "space": call.space().id,
    "kind": origin["kind"],
    "name": origin["name"],
    "services": origin["services"],
    "manage": origin["manage"],
    "access": if manages { vec!["manage"] } else { vec!["write"] },
    "sync": "off",
    "peers": [],
  })
}
