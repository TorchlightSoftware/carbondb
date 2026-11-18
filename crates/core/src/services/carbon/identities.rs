//! `POST [@carbon]/identity`, and the identities on this device as the system
//! database keeps them. An identity is its first device's key pair; its own
//! channel, running `[identity]` and sealed once founded, shared only by the
//! person's own devices; and its personal channel, where everything else of
//! the person's nests. They are created by `[@carbon]`'s effects and calls
//! (`spaces`), and set up by ops through `[channel]` and `[identity]`. Its
//! record here keeps the device's secret key, which no channel ever holds.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::steps::{self, Step};
use carbondb_svc_channel::names::slugify;
use carbondb_svc_identity::id_of;

use crate::error::{Result, bad_request, conflict, internal, not_found, single_node};
use crate::keys::{self, KeyPair};
use crate::calls::service_call::ServiceCall;
use carbondb_service_bindings::migrations::Migration;
use crate::{Actor, Db, HandlerContext};

// The system database's own tables, after a space's: the identities on this
// device, each with its first device's secret key, and the spaces it holds.
pub const SYSTEM_MIGRATIONS: &[Migration] = &[
  ("system/0001_identities", include_str!("../../../migrations/system/0001_identities.sql")),
];

// An identity on this device, as anyone on it may see it: everything but the
// secret key. `channel` is its own channel, `personal` its personal one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
  pub slug: String,
  pub id: String,
  pub channel: String,
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
  // This device's key, when it showed a code and the identity it joins has
  // arrived from the device that scanned it (`crate::nodes::pairing`).
  joining: Option<String>,
}

const COLUMNS: &str = "slug, id, channel, personal, device";

// An identity on this device, by its slug.
pub fn find(system: &dyn Db, slug: &str) -> Result<Identity> {
  one(system.get(&format!("SELECT {COLUMNS} FROM carbon_system_identity WHERE slug = ?"), &[json!(slug)])?, slug)
}

// An identity on this device, by its id: the identity a call acts as, with the device it holds here.
pub fn acting(system: &dyn Db, id: &str) -> Result<Identity> {
  one(system.get(&format!("SELECT {COLUMNS} FROM carbon_system_identity WHERE id = ?"), &[json!(id)])?, id)
}

fn one(row: Value, named: &str) -> Result<Identity> {
  if row.is_null() {
    return Err(not_found(format!("No identity '{named}' on this device")));
  }
  Ok(serde_json::from_value(row)?)
}

// The p2panda credentials of an identity's device here, which its node signs with.
pub fn credentials(system: &dyn Db, id: &str) -> Result<p2panda::Credentials> {
  let row = system.get("SELECT secret FROM carbon_system_identity WHERE id = ?", &[json!(id)])?;
  keys::credentials(row["secret"].as_str().ok_or_else(|| not_found(format!("No identity '{id}' on this device")))?)
}

pub fn list(system: &dyn Db) -> Result<Vec<Value>> {
  system.all(&format!("SELECT {COLUMNS} FROM carbon_system_identity ORDER BY slug"), &[])
}

impl crate::Carbon {
  // An identity's device credentials on this device, which `[@carbon]` signs with.
  pub(crate) fn credentials_of(&self, identity: &str) -> Result<p2panda::Credentials> {
    if let Some(credentials) = self.keys.borrow().get(identity) {
      return Ok(credentials.clone());
    }
    let read = credentials(&self.system.store().reader()?, identity)?;
    self.remember_keys(identity, read.clone());
    Ok(read)
  }

  // Keep an identity's credentials at hand, as founding does before the identity is recorded.
  pub(crate) fn remember_keys(&self, identity: &str, credentials: p2panda::Credentials) {
    self.keys.borrow_mut().insert(identity.to_string(), credentials);
  }

  // Let go of an identity's credentials, as a failed founding does.
  pub(crate) fn forget_keys(&self, identity: &str) {
    self.keys.borrow_mut().remove(identity);
  }
}

// Record an identity on this device, with its first device's secret key.
fn keep(system: &dyn Db, identity: &Identity, secret: &str) -> Result<()> {
  system.run(
    &format!("INSERT INTO carbon_system_identity ({COLUMNS}, secret) VALUES (?, ?, ?, ?, ?, ?)"),
    &[json!(identity.slug), json!(identity.id), json!(identity.channel), json!(identity.personal), json!(identity.device), json!(secret)],
  )?;
  Ok(())
}

// Found an identity, as one `[@carbon]` op on the system handle. When this
// device syncs, its node starts first, since it creates the p2panda space of
// each new channel. The `create` effect makes its own channel; then calls set
// it up: its origin, `[@carbon]` there adding `[identity]`, the identity's
// origin, and `[@carbon]` there adding the personal channel; then `[@carbon]`
// on the personal channel adds its services, and the own channel is sealed.
// The identity is recorded in this op's own transaction once all of it is
// set up, so an identity is listed only when it is whole.
pub fn create(call: Rc<ServiceCall>) {
  let request: IdentityInput = match serde_json::from_value(call.input.clone()) {
    Ok(request) => request,
    Err(error) => {
      let _ = call.settle(Err(error.into()));
      return;
    }
  };
  if let Some(device) = &request.joining {
    let joined = join(&call, device);
    let _ = call.settle(joined);
    return;
  }
  let founding = match plan(&call, request).and_then(|founding| start(&call, &founding).map(|_| founding)) {
    Ok(founding) => founding,
    Err(error) => {
      let _ = call.settle(Err(error));
      return;
    }
  };
  let caller = Rc::clone(&call);
  let own = json!({"package": "channel", "name": "channel", "identity": founding.id});
  HandlerContext::effect(call, "create", own, Box::new(move |created| match created.and_then(|created| created["space"].as_str().map(str::to_string).ok_or_else(|| internal("The create effect answered without its channel"))) {
    Ok(channel) => own_channel(caller, founding, channel),
    Err(error) => fail(&caller, &founding, error),
  }))
}

// Keep the new identity's keys at hand, for signing as it is set up, and start its node.
fn start(call: &ServiceCall, founding: &Founding) -> Result<()> {
  call.carbon().remember_keys(&founding.id, keys::credentials(&founding.keys.secret)?);
  call.carbon().start_founding_node(&founding.id, &founding.keys.secret)
}

// Set up the identity's own channel, and the personal channel inside it, whose ref the last step answers.
fn own_channel(call: Rc<ServiceCall>, founding: Founding, channel: String) {
  let caller = Rc::clone(&call);
  steps::run(call, founding.own_channel(&channel), Box::new(move |founded| {
    let personal = founded.and_then(|returned| returned.last().and_then(|reference| reference["channel"].as_str()).map(str::to_string).ok_or_else(|| internal("POST [@carbon]/channel answered without its channel")));
    match personal {
      Ok(personal) => finish(caller, founding, channel, personal),
      Err(error) => fail(&caller, &founding, error),
    }
  }))
}

// Add the personal channel's services and seal the own channel, then keep the identity here.
fn finish(call: Rc<ServiceCall>, founding: Founding, channel: String, personal: String) {
  let identity = founding.identity(&channel, &personal);
  let caller = Rc::clone(&call);
  steps::run(call, founding.rest(&channel, &personal), Box::new(move |result| {
    match result.and_then(|_| keep(caller.as_ref(), &identity, &founding.keys.secret)) {
      Ok(()) => {
        let _ = caller.settle(Ok(json!(identity)));
      }
      Err(error) => fail(&caller, &founding, error),
    }
  }))
}

// An identity that could not be founded: its node, if one started, stops,
// its keys are let go, and everything in its directory is deleted, the
// channels its calls created included.
fn fail(call: &ServiceCall, founding: &Founding, error: crate::ServiceError) {
  let carbon = call.carbon();
  carbon.stop_node(&founding.id);
  carbon.forget_keys(&founding.id);
  let error = carbon.forget_identity(&founding.id).err().unwrap_or(error);
  let _ = call.settle(Err(error));
}

// Record the identity a device here joined, with its key pair: its node becomes the identity's.
fn join(call: &ServiceCall, device: &str) -> Result<Value> {
  let carbon = call.carbon();
  let (secret, joined) = carbon.take_joined(device).ok_or_else(|| bad_request(format!("No device {device} here is waiting to join an identity")))?;
  if find(call, &joined.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", joined.slug)));
  }
  let identity = Identity { slug: joined.slug, id: joined.id, channel: joined.channel, personal: joined.personal, device: device.to_string() };
  keep(call, &identity, &secret)?;
  carbon.node_joins(device, &identity.id);
  Ok(json!(identity))
}

// An identity about to be founded: its slug, its first device's key pair and
// name, its id, and the services its personal channel runs.
struct Founding {
  slug: String,
  device_name: String,
  keys: KeyPair,
  id: String,
  personal_services: Vec<String>,
}

// Check an identity can be created, and plan it.
fn plan(call: &ServiceCall, request: IdentityInput) -> Result<Founding> {
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
  let keys = keys::generate()?;
  Ok(Founding { slug: request.slug, device_name: request.device, id: id_of(&keys.public), keys, personal_services: request.services })
}

impl Founding {
  // Acting as the new identity, from its first device.
  fn actor(&self) -> Actor {
    Actor { identity: Some(self.id.clone()), device: Some(self.keys.public.clone()) }
  }

  fn call(&self, space: &str, key: &'static str, input: Value) -> Step {
    Step { space: space.to_string(), actor: self.actor(), key, input }
  }

  // The own channel's origin, its `[identity]` with the first device, and the
  // personal channel inside it, added by `[@carbon]` there.
  fn own_channel(&self, own: &str) -> VecDeque<Step> {
    VecDeque::from([
      self.call(own, "POST [channel]/origin", json!({"kind": "identity", "name": self.slug, "manage": [self.id]})),
      self.call(own, "POST [@carbon]/services", json!({"name": "identity"})),
      self.call(own, "POST [identity]/origin", json!({"device": self.keys.public, "name": self.device_name, "tags": ["manage"]})),
      self.call(own, "POST [@carbon]/channel", json!({"kind": "personal", "name": self.slug, "slug": self.slug})),
    ])
  }

  // The personal channel's services, added by `[@carbon]` there, then the seal on the own channel.
  fn rest(&self, own: &str, personal: &str) -> VecDeque<Step> {
    let services = self.personal_services.iter().map(|name| self.call(personal, "POST [@carbon]/services", json!({"name": name})));
    services.chain(std::iter::once(self.call(own, "POST [channel]/seal", json!({})))).collect()
  }

  fn identity(&self, channel: &str, personal: &str) -> Identity {
    Identity { slug: self.slug.clone(), id: self.id.clone(), channel: channel.to_string(), personal: personal.to_string(), device: self.keys.public.clone() }
  }
}
