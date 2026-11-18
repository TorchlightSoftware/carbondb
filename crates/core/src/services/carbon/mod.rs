//! `[@carbon]`, the device's local service: host code that runs once, on this
//! device, and never replicates. It runs in local spaces: on the system
//! handle, in the system database, it founds and lists the identities here
//! and shows a code for this device to join one; on a channel's handle, in
//! the channel's local space, acting as that handle's identity, it adds a
//! service or a channel, enrolls a device, and reads the status of the
//! channel or of one service it names.
//!
//! It is part of the core, not a service like the others: it reaches keys,
//! space files, p2panda nodes and the system database directly, and only it
//! creates spaces (`spaces`). Everything it changes in a channel or service
//! goes through that one's own calls, made as effects of its own op, which is
//! logged in the local space it runs in, marked with this device. What a
//! service records only on `[@carbon]`'s word carries a signature by the op's
//! device key (`carbondb_service_bindings::signature`).

mod identities;
mod spaces;
mod steps;

use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{Result, bad_request, forbidden};
use crate::calls::service_call::ServiceCall;
use crate::services::Service;
use crate::channels::channel::Kind;
use crate::channels::service::RunState;
use crate::Carbon;
use crate::{Db, HandlerContext, Routing};

pub use identities::{Identity, SYSTEM_MIGRATIONS, acting, credentials, find};
pub(crate) use spaces::{create_effect, sign_effect};

// `[@carbon]` as the core registers it. It is not a package: no peer checks
// its version, so its version is the core's own.
pub fn service() -> Service {
  Service {
    name: "carbon".into(),
    version: concat!("carbondb@", env!("CARGO_PKG_VERSION")).into(),
    binding: None,
    timeout: crate::services::DEFAULT_TIMEOUT,
    always: vec![],
    health: None,
    startup: crate::services::DEFAULT_STARTUP_TIMEOUT,
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
    ("POST", "/devices") => Rc::clone(call.carbon()).show_code(Rc::clone(&call)),
    ("POST", "/enroll") => Rc::clone(call.carbon()).enroll(Rc::clone(&call)),
    ("POST", "/services") => spaces::add_service(Rc::clone(&call)),
    ("POST", "/channel") => spaces::add_channel(Rc::clone(&call)),
    ("GET", "/identities") => answer(identities::list(call.as_ref()).map(|identities| json!(identities))),
    ("GET", "/status") => answer(target(&call).and_then(|target| status(&call, &target))),
    ("POST", "/sync/start" | "/sync/stop") => {
      let input = call.input.clone();
      let syncing = Rc::clone(&call);
      HandlerContext::effect(Rc::clone(&call), "sync", input, Box::new(move |synced| {
        let _ = syncing.settle(synced);
      }))
    }
    _ => answer(Err(call.call.unknown())),
  }
}

// The channel whose handle a call was made on: its `[@carbon]`'s; the system has none to report on.
fn channel(call: &ServiceCall) -> Result<String> {
  match call.space().channel.kind {
    Kind::Replicated => Ok(call.space().channel.id.clone()),
    Kind::Local => Err(bad_request("Make this call on a channel's handle")),
  }
}

// The identity a call acts as, which a call that changes a channel needs.
fn acting_identity(call: &ServiceCall) -> Result<String> {
  call.actor.identity.clone().ok_or_else(|| forbidden(format!("{} needs an acting identity", call.call.key())))
}

// What a call about this device's copy is about: the channel whose handle it
// was made on, and one service that channel names, or the channel itself.
struct Target {
  channel: String,
  service: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Naming {
  service: Option<String>,
}

fn target(call: &ServiceCall) -> Result<Target> {
  let channel = channel(call)?;
  let naming: Option<Naming> = serde_json::from_value(call.input.clone())?;
  Ok(Target { channel, service: naming.unwrap_or_default().service })
}

// A channel's or a service's state on this device, for the acting
// identity: where it is in its lifecycle, and the access the identity has.
// Single-node: sync is off and there are no peers.
fn status(call: &ServiceCall, target: &Target) -> Result<Value> {
  let carbon = call.carbon();
  let channel = target.channel.clone();
  let access = access(call, &channel)?;
  if let Some(name) = &target.service {
    let state = lifecycle(carbon, &channel, name)?;
    return Ok(with_state(json!({"name": name, "access": access, "sync": "off", "peers": []}), state));
  }
  let origin = carbon.read_tables(&channel, |db| db.get("SELECT kind, name FROM origin", &[]))?;
  let named = carbon.read_tables(&channel, |db| db.all("SELECT name FROM service ORDER BY name", &[]))?;
  let services = named.iter().map(|service| Ok(with_state(json!({"name": service["name"], "access": access, "sync": "off"}), lifecycle(carbon, &channel, service["name"].as_str().unwrap_or_default())?))).collect::<Result<Vec<Value>>>()?;
  let status = json!({"channel": channel, "kind": origin["kind"], "name": origin["name"], "access": access, "sync": "off", "peers": [], "services": services});
  Ok(with_state(status, lifecycle(carbon, &channel, "channel")?))
}

// Where a service a channel names is in its lifecycle on this device, opening
// its store to see: `{state}`, initializing, idle, applying or stopped, and
// `error`, why it stopped.
fn lifecycle(carbon: &Rc<Carbon>, channel: &str, name: &str) -> Result<Value> {
  let store = carbon.named_store(&carbon.channel(channel)?, name)?;
  Ok(match store.run_state() {
    RunState::Stopped { error } => json!({"state": "stopped", "error": error}),
    state => json!({"state": json!(state)["state"]}),
  })
}

fn with_state(mut status: Value, state: Value) -> Value {
  if let (Some(status), Value::Object(state)) = (status.as_object_mut(), state) {
    status.extend(state);
  }
  status
}

// The acting identity's access to a channel's services, on this device:
// manage for a manager, write for anyone else acting, none for no one.
fn access(call: &ServiceCall, channel: &str) -> Result<Value> {
  let Some(identity) = &call.actor.identity else { return Ok(json!([])) };
  let manage = call.carbon().read_tables(channel, |db| db.get("SELECT manage FROM origin", &[]))?;
  let managers: Vec<String> = serde_json::from_str(manage["manage"].as_str().unwrap_or("[]"))?;
  Ok(json!(if managers.contains(identity) { ["manage"] } else { ["write"] }))
}
