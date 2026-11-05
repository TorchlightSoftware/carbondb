//! `[@carbon]`, the device's local service: host code that runs once, on this
//! device, and never replicates. It answers on the system handle, where
//! identities are created and listed, and in every space, where spaces are
//! created and the space's status is read. Calls that need peers, or that are
//! designed but not built, are refused here.
//!
//! What it sets up in replicated spaces it writes as ops there, through the
//! services that own those records, so every replica sees them. Its own call is
//! logged where it ran, marked with this device. Setting something up is a list
//! of steps, run in order, stopping at the first that fails.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::space;
use crate::call::{Actor, Call};
use crate::error::{Result, bad_request, conflict, not_built, single_node};
use crate::hash::hash;
use crate::identity::{self, Identity};
use crate::invocation::Invocation;
use crate::keys;
use crate::names::slugify;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityInput {
  slug: String,
  device: String,
  #[serde(default)]
  services: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceInput {
  #[serde(default = "default_kind")]
  kind: String,
  name: String,
  #[serde(default)]
  services: Vec<String>,
  manage: Option<Vec<String>>,
  #[serde(default)]
  cascade: Vec<String>,
  slug: Option<String>,
}

fn default_kind() -> String {
  "space".to_string()
}

pub fn entry(invocation: Rc<Invocation>) {
  let root = invocation.space().is_root();
  let answer = |result: Result<Value>| {
    let _ = invocation.settle(result);
  };
  match (root, invocation.call.method(), invocation.call.resource.as_str()) {
    (true, "POST", "/identity") => create_identity(Rc::clone(&invocation)),
    (true, "GET", "/identities") => answer(identity::list(invocation.as_ref()).map(|identities| json!(identities))),
    (true, "POST", "/devices") => answer(Err(single_node("Adding a device"))),
    (false, "GET", "/status") => answer(status(&invocation)),
    (false, "POST", "/space") => create_space(Rc::clone(&invocation)),
    (false, "GET", "/manage") => answer(space::managers(invocation.as_ref()).map(|managers| json!(managers))),
    (false, _, "/members" | "/join" | "/sync/start" | "/sync/stop") => answer(Err(single_node(&invocation.call.key()))),
    (false, "POST", "/manage") => answer(Err(not_built("Changing a manage list"))),
    (false, "DELETE", "/data") => answer(Err(not_built("Deleting a space's data"))),
    _ => answer(Err(invocation.call.unknown())),
  }
}

// Where a step's call runs: the space `[@carbon]` was called in, or another by id.
#[derive(Debug, Clone, PartialEq)]
enum Target {
  Here,
  Space(String),
}

// One step of setting something up.
#[derive(Debug, Clone, PartialEq)]
enum Step {
  // A new space's database, on this device.
  Create(String),
  // A call, as an op of its own in its space.
  Call { target: Target, actor: Actor, key: &'static str, input: Value },
  // Provision the services a space's origin named, once that origin applies.
  Provision(String, Vec<String>),
}

// What the steps' calls returned, in order, or the first error.
type Done = Box<dyn FnOnce(Result<Vec<Value>>)>;

// Run steps in order, each once the one before it has applied.
fn run(invocation: Rc<Invocation>, mut steps: VecDeque<Step>, mut returned: Vec<Value>, done: Done) {
  let Some(step) = steps.pop_front() else { return done(Ok(returned)) };
  let carbon = Rc::clone(invocation.carbon());
  let host = match step {
    Step::Create(id) => carbon.create_space(&id).map(|_| ()),
    Step::Provision(id, services) => carbon.space(&id).and_then(|space| carbon.provision(&space, &services)),
    Step::Call { target, actor, key, input } => {
      let space = match target {
        Target::Here => Ok(Rc::clone(invocation.space())),
        Target::Space(id) => carbon.space(&id),
      };
      let (space, call) = match space.and_then(|space| Ok((space, Call::from_key(key)?))) {
        Ok(found) => found,
        Err(error) => return done(Err(error)),
      };
      let next = Rc::clone(&invocation);
      return invocation.call_in(&space, actor, call, input, Box::new(move |result| match result {
        Ok(value) => {
          returned.push(value);
          run(next, steps, returned, done)
        }
        Err(error) => done(Err(error)),
      }));
    }
  };
  match host {
    Ok(()) => run(invocation, steps, returned, done),
    Err(error) => done(Err(error)),
  }
}

// Create an identity on this device: its first device's key pair, its identity
// and personal spaces, each set up by ops, and its record in the system
// database, written in this call's own transaction, so an identity is listed
// only once both its spaces are set up.
fn create_identity(invocation: Rc<Invocation>) {
  let (identity, secret, steps) = match plan_identity(&invocation) {
    Ok(planned) => planned,
    Err(error) => {
      let _ = invocation.settle(Err(error));
      return;
    }
  };
  let caller = Rc::clone(&invocation);
  run(invocation, steps, vec![], Box::new(move |result| {
    let kept = result.and_then(|_| identity::keep(caller.as_ref(), &identity, &secret)).map(|_| json!(identity));
    let _ = caller.settle(kept);
  }));
}

// Check an identity can be created, and plan its setup. Both spaces are named
// by this call's op, so a retry after a crash reaches the same spaces.
fn plan_identity(invocation: &Invocation) -> Result<(Identity, String, VecDeque<Step>)> {
  let request: IdentityInput = serde_json::from_value(invocation.input.clone())?;
  if request.slug.is_empty() || slugify(&request.slug) != request.slug {
    return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", request.slug)));
  }
  if identity::find(invocation, &request.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", request.slug)));
  }
  invocation.carbon().services().require_registered(&request.services)?;
  let op = invocation.op().map(|op| op.hash.clone()).expect("a POST is an op");
  let keys = keys::generate()?;
  let identity = Identity {
    slug: request.slug,
    id: identity::id_of(&keys.public),
    space: hash(&json!({"op": op, "space": "identity"})),
    personal: hash(&json!({"op": op, "space": "personal"})),
    device: keys.public,
  };
  let personal_services = [vec!["device".to_string()], request.services.into_iter().filter(|service| service != "device").collect()].concat();
  let steps = identity_steps(&identity, &request.device, personal_services);
  Ok((identity, keys.secret, steps))
}

// An identity's setup, acting as the new identity from its first device.
fn identity_steps(identity: &Identity, device_name: &str, personal_services: Vec<String>) -> VecDeque<Step> {
  let actor = Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) };
  let call = |space: &str, key: &'static str, input: Value| Step::Call { target: Target::Space(space.to_string()), actor: actor.clone(), key, input };
  let identity_services = vec!["identity".to_string(), "device".to_string()];
  VecDeque::from([
    Step::Create(identity.space.clone()),
    call(&identity.space, "POST [space]/origin", json!({
      "kind": "identity",
      "name": identity.slug,
      "services": identity_services,
      "manage": ["self"],
    })),
    Step::Provision(identity.space.clone(), identity_services),
    call(&identity.space, "POST [identity]/origin", json!({"device": identity.device})),
    call(&identity.space, "POST [device]/origin", json!({"pubkey": identity.device})),
    Step::Create(identity.personal.clone()),
    call(&identity.personal, "POST [space]/origin", json!({
      "kind": "personal",
      "name": identity.slug,
      "services": personal_services,
      "manage": [identity.id],
    })),
    Step::Provision(identity.personal.clone(), personal_services.clone()),
    call(&identity.personal, "POST [device]/ref", json!({"pubkey": identity.device, "name": device_name, "tags": ["manage"]})),
    call(&identity.space, "POST [space]/ref", json!({
      "space": identity.personal,
      "kind": "personal",
      "name": identity.slug,
      "slug": identity.slug,
    })),
  ])
}

// Create a space nested in the calling one. The calling space must accept the
// ref first, so a caller without manage there creates nothing. The id is this
// call's op hash, so a retry after a crash reaches the same space.
fn create_space(invocation: Rc<Invocation>) {
  let steps = match plan_space(&invocation) {
    Ok(steps) => steps,
    Err(error) => {
      let _ = invocation.settle(Err(error));
      return;
    }
  };
  let caller = Rc::clone(&invocation);
  run(invocation, steps, vec![], Box::new(move |result| {
    let reference = result.map(|mut returned| returned.swap_remove(0));
    let _ = caller.settle(reference);
  }));
}

fn plan_space(invocation: &Invocation) -> Result<VecDeque<Step>> {
  let actor = invocation.actor.clone();
  let author = actor.identity.clone().ok_or_else(|| bad_request("Creating a space needs an acting identity"))?;
  let request: SpaceInput = serde_json::from_value(invocation.input.clone())?;
  invocation.carbon().services().require_registered(&request.services)?;
  let id = invocation.op().map(|op| op.hash.clone()).expect("a POST is an op");
  let origin = json!({
    "kind": request.kind,
    "name": request.name,
    "services": request.services,
    "manage": request.manage.unwrap_or_else(|| vec![author]),
  });
  let reference = json!({
    "space": id,
    "kind": request.kind,
    "name": request.name,
    "slug": request.slug,
    "cascade": request.cascade,
  });
  Ok(VecDeque::from([
    Step::Call { target: Target::Here, actor: actor.clone(), key: "POST [space]/ref", input: reference },
    Step::Create(id.clone()),
    Step::Call { target: Target::Space(id.clone()), actor, key: "POST [space]/origin", input: origin },
    Step::Provision(id, request.services),
  ]))
}

// What a space is, and its state on this device. Single-node: sync is off, there
// are no peers, and every identity here has full access.
fn status(invocation: &Invocation) -> Result<Value> {
  let origin = space::origin(invocation)?;
  let managers = space::managers(invocation)?;
  let manages = invocation.actor.identity.as_ref().is_some_and(|identity| managers.contains(identity));
  Ok(json!({
    "space": invocation.space().id,
    "kind": origin.kind,
    "name": origin.name,
    "services": origin.services,
    "manage": origin.manage,
    "access": if manages { vec!["manage"] } else { vec!["write"] },
    "sync": "off",
    "peers": [],
  }))
}
