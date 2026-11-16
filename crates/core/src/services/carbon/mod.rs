//! `[@carbon]`, the device's local service: host code that runs once, on this
//! device, and never replicates. It runs in the system: on the system handle
//! it founds and lists the identities here; on a channel's handle, acting as
//! that handle's identity, it reads the status of, syncs, or deletes this
//! device's copy of the channel, or of one service it names.
//!
//! It is part of the core, not a service like the others: it reaches keys,
//! space files and the system database directly. Everything it changes in a
//! channel or service goes through that one's own calls, made as effects of
//! its own op, which is logged in the system log, marked with this device.

mod identities;
mod steps;

use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Result, bad_request};
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
    version: concat!("carbondb@", env!("CARGO_PKG_VERSION")).into(),
    binding: None,
    timeout: crate::services::DEFAULT_TIMEOUT,
    always: vec![],
    setup: Rc::new(|_: Rc<dyn Db>| Ok(())),
    entry: Rc::new(entry),
  }
}

fn entry(call: Rc<ServiceCall>) {
  let answer = |result: Result<Value>| {
    let _ = call.settle(result);
  };
  match (call.call.method(), call.call.resource.as_str()) {
    ("POST", "/identity") => identities::create(Rc::clone(&call)),
    ("GET", "/identities") => answer(identities::list(call.as_ref()).map(|identities| json!(identities))),
    ("GET", "/status") => answer(target(&call).and_then(|target| status(&call, &target))),
    ("POST", "/sync/start" | "/sync/stop") => {
      let input = call.input.clone();
      let syncing = Rc::clone(&call);
      HandlerContext::effect(Rc::clone(&call), "sync", input, Box::new(move |synced| {
        let _ = syncing.settle(synced);
      }))
    }
    ("DELETE", "/data") => answer(target(&call).and_then(|target| delete(&call, &target))),
    _ => answer(Err(call.call.unknown())),
  }
}

// What a call about this device's copy is about: the channel whose handle it
// was made on, and one service that channel names, or the channel itself.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
  channel: Option<String>,
  service: Option<String>,
}

fn target(call: &ServiceCall) -> Result<Target> {
  let target: Target = serde_json::from_value(call.input.clone())?;
  match target.channel {
    Some(_) => Ok(target),
    None => Err(bad_request("Make this call on a channel's handle")),
  }
}

// A channel's or a service's state on this device, for the acting
// identity. Single-node: sync is off and there are no peers.
fn status(call: &ServiceCall, target: &Target) -> Result<Value> {
  let carbon = call.carbon();
  let channel = target.channel.clone().unwrap_or_default();
  carbon.space(&channel)?;
  let access = access(call, &channel)?;
  if let Some(name) = &target.service {
    carbon.space(&carbon.named(&channel, name)?)?;
    return Ok(json!({"name": name, "access": access, "sync": "off", "peers": []}));
  }
  let origin = carbon.read_tables(&channel, |db| db.get("SELECT kind, name FROM origin", &[]))?;
  let services = carbon.read_tables(&channel, |db| db.all("SELECT name FROM service ORDER BY name", &[]))?;
  let services: Vec<Value> = services.iter().map(|service| json!({"name": service["name"], "access": access, "sync": "off"})).collect();
  Ok(json!({"channel": channel, "kind": origin["kind"], "name": origin["name"], "access": access, "sync": "off", "peers": [], "services": services}))
}

// The acting identity's access to a channel's services, on this device:
// manage for a manager, write for anyone else acting, none for no one.
fn access(call: &ServiceCall, channel: &str) -> Result<Value> {
  let Some(identity) = &call.actor.identity else { return Ok(json!([])) };
  let manage = call.carbon().read_tables(channel, |db| db.get("SELECT manage FROM origin", &[]))?;
  let managers: Vec<String> = serde_json::from_str(manage["manage"].as_str().unwrap_or("[]"))?;
  Ok(json!(if managers.contains(identity) { ["manage"] } else { ["write"] }))
}

// Delete this device's copy of a service the channel names, or of the channel and its services.
fn delete(call: &ServiceCall, target: &Target) -> Result<Value> {
  let carbon = call.carbon();
  let channel = target.channel.clone().unwrap_or_default();
  let held = match &target.service {
    Some(name) => vec![carbon.named(&channel, name)?],
    None => {
      carbon.space(&channel)?;
      let services = carbon.read_tables(&channel, |db| db.all("SELECT space FROM service", &[]))?;
      services.iter().filter_map(|service| service["space"].as_str().map(str::to_string)).chain(std::iter::once(channel)).collect()
    }
  };
  held.iter().try_for_each(|id| carbon.forget(id))?;
  Ok(json!({}))
}
