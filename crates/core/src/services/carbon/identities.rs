//! `POST [@carbon]/identity`, and the identities on this device as the system
//! database keeps them. An identity is its first device's key pair, an identity
//! space and a personal space, each set up by ops through `[space]`,
//! `[identity]` and `[device]`, and its record here, with the first device's
//! secret key, which no space ever holds.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::steps::{self, Step, Target};
use crate::error::{Result, bad_request, conflict, not_found};
use crate::frame::Frame;
use crate::hash::hash;
use crate::keys;
use crate::names::slugify;
use crate::store::Store;
use crate::{Actor, Db, Invocation};

const SYSTEM_SCHEMA: &str = include_str!("../../../migrations/system.sql");

// An identity on this device, as anyone on it may see it: everything but the secret key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
  pub slug: String,
  pub id: String,
  pub space: String,
  pub personal: String,
  pub device: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityInput {
  slug: String,
  device: String,
  #[serde(default)]
  services: Vec<String>,
}

// The system database: this device's identities.
pub fn open_system(path: &std::path::Path) -> Result<Store> {
  let system = Store::open(path)?;
  system.run_batch(SYSTEM_SCHEMA)?;
  Ok(system)
}

pub fn find(system: &dyn Db, slug: &str) -> Result<Identity> {
  let row = system.get("SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?;
  if row.is_null() {
    return Err(not_found(format!("No identity '{slug}' on this device")));
  }
  Ok(serde_json::from_value(row)?)
}

pub fn list(system: &dyn Db) -> Result<Vec<Value>> {
  system.all("SELECT slug, id, space, personal, device FROM carbon_system_identity ORDER BY slug", &[])
}

// Record an identity on this device, with its first device's secret key.
fn keep(system: &dyn Db, identity: &Identity, secret: &str) -> Result<()> {
  system.run(
    "INSERT INTO carbon_system_identity (slug, id, space, personal, device, secret) VALUES (?, ?, ?, ?, ?, ?)",
    &[json!(identity.slug), json!(identity.id), json!(identity.space), json!(identity.personal), json!(identity.device), json!(secret)],
  )?;
  Ok(())
}

// Create an identity, recording it in this call's own transaction once both
// its spaces are set up, so an identity is listed only when it is whole.
pub fn create(frame: Rc<Frame>) {
  let planned = plan(&frame);
  let caller = Rc::clone(&frame);
  match planned {
    Ok((identity, secret, planned)) => steps::run(frame, planned, Box::new(move |result| {
      let kept = result.and_then(|_| keep(caller.as_ref(), &identity, &secret)).map(|_| json!(identity));
      let _ = caller.settle(kept);
    })),
    Err(error) => {
      let _ = caller.settle(Err(error));
    }
  }
}

// Check an identity can be created, and plan its setup. Both spaces are named
// by this call's op, so a retry after a crash reaches the same spaces.
fn plan(frame: &Frame) -> Result<(Identity, String, VecDeque<Step>)> {
  let request: IdentityInput = serde_json::from_value(frame.input.clone())?;
  if request.slug.is_empty() || slugify(&request.slug) != request.slug {
    return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", request.slug)));
  }
  if find(frame, &request.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", request.slug)));
  }
  frame.carbon().services().require_registered(&request.services)?;
  let op = frame.op().map(|op| op.hash.clone()).expect("a POST is an op");
  let keys = keys::generate()?;
  let identity = Identity {
    slug: request.slug,
    id: carbondb_identity::id_of(&keys.public),
    space: hash(&json!({"op": op, "space": "identity"})),
    personal: hash(&json!({"op": op, "space": "personal"})),
    device: keys.public,
  };
  let personal_services = [vec!["device".to_string()], request.services.into_iter().filter(|service| service != "device").collect()].concat();
  let planned = setup(&identity, &request.device, personal_services);
  Ok((identity, keys.secret, planned))
}

// An identity's setup, acting as the new identity from its first device.
fn setup(identity: &Identity, device_name: &str, personal_services: Vec<String>) -> VecDeque<Step> {
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
