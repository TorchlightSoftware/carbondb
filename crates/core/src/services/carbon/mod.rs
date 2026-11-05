//! `[@carbon]`, the device's local service: host code that runs once, on this
//! device, and never replicates. It answers on the system handle, where
//! identities are created and listed, and in every space, where spaces are
//! created and the space's status is read. Calls that need peers, or that are
//! designed but not built, are refused here.
//!
//! It is the one service the core implements itself, since it reaches keys,
//! space files and, later, p2panda. Everything else it learns or changes in a
//! replicated space goes through that space's services: reads as calls, and
//! writes as ops there, so every replica sees them. Its own call is logged
//! where it ran, marked with this device.

mod identities;
mod spaces;
mod steps;

use std::rc::Rc;

use serde_json::{Value, json};

use crate::error::{Result, not_built, single_node};
use crate::frame::Frame;
use crate::{Invocation, ask};

pub use identities::{Identity, find, open_system};

pub fn entry(frame: Rc<Frame>) {
  let root = frame.space().is_root();
  let answer = |result: Result<Value>| {
    let _ = frame.settle(result);
  };
  match (root, frame.call.method(), frame.call.resource.as_str()) {
    (true, "POST", "/identity") => identities::create(Rc::clone(&frame)),
    (true, "GET", "/identities") => answer(identities::list(frame.as_ref()).map(|identities| json!(identities))),
    (true, "POST", "/devices") => answer(Err(single_node("Adding a device"))),
    (false, "GET", "/status") => answer(status(&frame)),
    (false, "POST", "/space") => spaces::create(Rc::clone(&frame)),
    (false, "GET", "/manage") => answer(ask(&invocation(&frame), "GET [space]/manage", Value::Null)),
    (false, _, "/members" | "/join" | "/sync/start" | "/sync/stop") => answer(Err(single_node(&frame.call.key()))),
    (false, "POST", "/manage") => answer(Err(not_built("Changing a manage list"))),
    (false, "DELETE", "/data") => answer(Err(not_built("Deleting a space's data"))),
    _ => answer(Err(frame.call.unknown())),
  }
}

// A frame as any service sees it, for asking the space's services.
fn invocation(frame: &Rc<Frame>) -> Rc<dyn Invocation> {
  Rc::clone(frame) as Rc<dyn Invocation>
}

// What a space is, and its state on this device. Single-node: sync is off, there
// are no peers, and every identity here has full access.
fn status(frame: &Rc<Frame>) -> Result<Value> {
  let origin = ask(&invocation(frame), "GET [space]/origin", Value::Null)?;
  let managers = ask(&invocation(frame), "GET [space]/manage", Value::Null)?;
  let manages = frame.actor.identity.as_ref().is_some_and(|identity| managers.as_array().is_some_and(|managers| managers.contains(&json!(identity))));
  Ok(json!({
    "space": frame.space().id,
    "kind": origin["kind"],
    "name": origin["name"],
    "services": origin["services"],
    "manage": origin["manage"],
    "access": if manages { vec!["manage"] } else { vec!["write"] },
    "sync": "off",
    "peers": [],
  }))
}
