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
use carbondb_space::names::slugify;

use crate::error::{Result, bad_request, conflict, internal, not_found, single_node};
use crate::hash::hash;
use crate::keys::{self, KeyPair};
use crate::service_call::ServiceCall;
use carbondb_service_bindings::migrations::Migration;
use crate::{Actor, Db, HandlerContext};

// The system database's own tables, after a space's: the identities on this
// device, each with its first device's secret key.
pub const SYSTEM_MIGRATIONS: &[Migration] = &[("system/0001_identities", include_str!("../../../migrations/system/0001_identities.sql"))];

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
  // A code scanned from a device already in the identity, to join it rather than found one.
  code: Option<String>,
}


// An identity on this device, by its slug.
pub fn find(system: &dyn Db, slug: &str) -> Result<Identity> {
  one(system.get("SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?, slug)
}

// An identity on this device, by its id: the identity a call acts as, with the device it holds here.
pub fn acting(system: &dyn Db, id: &str) -> Result<Identity> {
  one(system.get("SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE id = ?", &[json!(id)])?, id)
}

fn one(row: Value, named: &str) -> Result<Identity> {
  if row.is_null() {
    return Err(not_found(format!("No identity '{named}' on this device")));
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
// its spaces are set up, so an identity is listed only when it is whole. Its
// identity space is founded by its first device alone, since the identity's id
// exists only once `POST [identity]/origin` answers with it; every op after
// carries the id.
pub fn create(call: Rc<ServiceCall>) {
  let founding = match plan(&call) {
    Ok(founding) => founding,
    Err(error) => {
      let _ = call.settle(Err(error));
      return;
    }
  };
  let caller = Rc::clone(&call);
  steps::run(call, founding.steps(), Box::new(move |founded| {
    let identity = founded.and_then(|returned| founding.identity(returned.last()));
    match identity {
      Ok(identity) => set_up(caller, founding, identity),
      Err(error) => {
        let _ = caller.settle(Err(error));
      }
    }
  }))
}

// Set up the rest as the new identity, then keep it on this device.
fn set_up(call: Rc<ServiceCall>, founding: Founding, identity: Identity) {
  let caller = Rc::clone(&call);
  steps::run(call, founding.setup(&identity), Box::new(move |result| {
    let kept = result.and_then(|_| keep(caller.as_ref(), &identity, &founding.keys.secret)).map(|_| json!(identity));
    let _ = caller.settle(kept);
  }))
}

// An identity about to be founded: its slug, its first device's key pair and
// name, the ids of its two spaces, and the services its personal space runs.
struct Founding {
  slug: String,
  device_name: String,
  keys: KeyPair,
  space: String,
  personal: String,
  personal_services: Vec<String>,
}

// Check an identity can be created, and plan it. Both spaces are named by this
// call's op, so a retry after a crash reaches the same spaces.
fn plan(call: &ServiceCall) -> Result<Founding> {
  let request: IdentityInput = serde_json::from_value(call.input.clone())?;
  if request.code.is_some() {
    return Err(single_node("Joining an identity from another device"));
  }
  if request.slug.is_empty() || slugify(&request.slug) != request.slug {
    return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", request.slug)));
  }
  if find(call, &request.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", request.slug)));
  }
  call.carbon().services().require_registered(&request.services)?;
  let op = call.op().map(|op| op.hash.clone()).expect("a POST is an op");
  Ok(Founding {
    slug: request.slug,
    device_name: request.device,
    keys: keys::generate()?,
    space: hash(&json!({"op": op, "space": "identity"})),
    personal: hash(&json!({"op": op, "space": "personal"})),
    personal_services: [vec!["device".to_string()], request.services.into_iter().filter(|service| service != "device").collect()].concat(),
  })
}

impl Founding {
  // The identity space and its origins, written by the first device alone.
  fn steps(&self) -> VecDeque<Step> {
    let device = Actor { identity: None, device: Some(self.keys.public.clone()) };
    let call = |key: &'static str, input: Value| Step::Call { target: Target::Space(self.space.clone()), actor: device.clone(), key, input };
    let services = vec!["identity".to_string(), "device".to_string()];
    VecDeque::from([
      Step::Create(self.space.clone()),
      call("POST [space]/origin", json!({"kind": "identity", "name": self.slug, "services": services, "manage": ["self"]})),
      Step::SetUp(self.space.clone(), services),
      call("POST [identity]/origin", json!({"device": self.keys.public})),
    ])
  }

  // The identity, with the id `POST [identity]/origin` answered.
  fn identity(&self, origin: Option<&Value>) -> Result<Identity> {
    let id = origin.and_then(|origin| origin["id"].as_str()).ok_or_else(|| internal("POST [identity]/origin answered without an id"))?;
    Ok(Identity { slug: self.slug.clone(), id: id.to_string(), space: self.space.clone(), personal: self.personal.clone(), device: self.keys.public.clone() })
  }

  // The rest of the setup, acting as the new identity from its first device.
  fn setup(&self, identity: &Identity) -> VecDeque<Step> {
    let actor = Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) };
    let call = |space: &str, key: &'static str, input: Value| Step::Call { target: Target::Space(space.to_string()), actor: actor.clone(), key, input };
    VecDeque::from([
      call(&self.space, "POST [device]/origin", json!({"pubkey": identity.device})),
      Step::Create(self.personal.clone()),
      call(&self.personal, "POST [space]/origin", json!({
        "kind": "personal",
        "name": self.slug,
        "services": self.personal_services,
        "manage": [identity.id],
      })),
      Step::SetUp(self.personal.clone(), self.personal_services.clone()),
      call(&self.personal, "POST [device]/ref", json!({"pubkey": identity.device, "name": self.device_name, "tags": ["manage"]})),
      call(&self.space, "POST [space]/ref", json!({"space": self.personal, "kind": "personal", "name": self.slug, "slug": self.slug})),
    ])
  }
}
