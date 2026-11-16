//! `POST [@carbon]/identity`, and the identities on this device as the system
//! database keeps them. An identity is its first device's key pair; its own
//! channel, running `[identity]` and sealed once founded, shared only by the
//! person's own devices; and its personal channel, where everything else of
//! the person's nests. Both are set up by ops through `[channel]` and
//! `[identity]`. Its record here keeps the first device's secret key, which no
//! channel ever holds.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::steps::{self, Step};
use crate::services::channel::names::slugify;
use crate::services::identity::id_of;

use crate::error::{Result, bad_request, conflict, internal, not_found, single_node};
use crate::hash::hash;
use crate::keys::{self, KeyPair};
use crate::service_call::ServiceCall;
use carbondb_service_bindings::migrations::Migration;
use crate::{Actor, Db, HandlerContext};

// The system database's own tables, after a space's: the identities on this
// device, each with its first device's secret key, and the spaces it holds.
pub const SYSTEM_MIGRATIONS: &[Migration] = &[
  ("system/0001_identities", include_str!("../../../migrations/system/0001_identities.sql")),
  ("system/0002_channels_and_spaces", include_str!("../../../migrations/system/0002_channels_and_spaces.sql")),
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

pub fn list(system: &dyn Db) -> Result<Vec<Value>> {
  system.all(&format!("SELECT {COLUMNS} FROM carbon_system_identity ORDER BY slug"), &[])
}

// Record an identity on this device, with its first device's secret key.
fn keep(system: &dyn Db, identity: &Identity, secret: &str) -> Result<()> {
  system.run(
    &format!("INSERT INTO carbon_system_identity ({COLUMNS}, secret) VALUES (?, ?, ?, ?, ?, ?)"),
    &[json!(identity.slug), json!(identity.id), json!(identity.channel), json!(identity.personal), json!(identity.device), json!(secret)],
  )?;
  Ok(())
}

// Found an identity: its own channel, then its personal channel and the
// services it runs, then the seal on its own channel; recorded in this call's
// own transaction once all of it is set up, so an identity is listed only when
// it is whole.
pub fn create(call: Rc<ServiceCall>) {
  let founding = match plan(&call) {
    Ok(founding) => founding,
    Err(error) => {
      let _ = call.settle(Err(error));
      return;
    }
  };
  let caller = Rc::clone(&call);
  steps::run(call, founding.own_channel(), Box::new(move |founded| {
    let personal = founded.and_then(|returned| returned.last().and_then(|reference| reference["channel"].as_str()).map(str::to_string).ok_or_else(|| internal("POST [channel]/channel answered without its channel")));
    match personal {
      Ok(personal) => finish(caller, founding, personal),
      Err(error) => {
        let _ = caller.settle(Err(error));
      }
    }
  }))
}

// Add the personal channel's services and seal the identity's own channel, then keep the identity here.
fn finish(call: Rc<ServiceCall>, founding: Founding, personal: String) {
  let identity = founding.identity(&personal);
  let caller = Rc::clone(&call);
  steps::run(call, founding.rest(&personal), Box::new(move |result| {
    let kept = result.and_then(|_| keep(caller.as_ref(), &identity, &founding.keys.secret)).map(|_| json!(identity));
    let _ = caller.settle(kept);
  }))
}

// An identity about to be founded: its slug, its first device's key pair and
// name, its id, its own channel's id, and the services its personal channel runs.
struct Founding {
  slug: String,
  device_name: String,
  keys: KeyPair,
  id: String,
  channel: String,
  personal_services: Vec<String>,
}

// Check an identity can be created, and plan it. Its own channel is named by
// this call's op, so a retry after a crash reaches the same channel.
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
  let keys = keys::generate()?;
  Ok(Founding {
    slug: request.slug,
    device_name: request.device,
    id: id_of(&keys.public),
    keys,
    channel: hash(&json!({"op": op, "channel": "identity"})),
    personal_services: request.services,
  })
}

impl Founding {
  // Acting as the new identity, from its first device.
  fn actor(&self) -> Actor {
    Actor { identity: Some(self.id.clone()), device: Some(self.keys.public.clone()) }
  }

  // The identity's own channel, its `[identity]` with the first device, and
  // the personal channel inside it, whose ref the last step answers.
  fn own_channel(&self) -> VecDeque<Step> {
    let call = |key: &'static str, input: Value| Step::Call { space: self.channel.clone(), actor: self.actor(), key, input };
    VecDeque::from([
      Step::Effect { name: "create", input: json!({"space": self.channel, "package": "channel", "name": "channel"}) },
      call("POST [channel]/origin", json!({"kind": "identity", "name": self.slug, "manage": [self.id]})),
      call("POST [channel]/services", json!({"name": "identity"})),
      call("POST [identity]/origin", json!({"device": self.keys.public, "name": self.device_name, "tags": ["manage"]})),
      call("POST [channel]/channel", json!({"kind": "personal", "name": self.slug, "slug": self.slug})),
    ])
  }

  // The personal channel's services, then the seal on the identity's own channel.
  fn rest(&self, personal: &str) -> VecDeque<Step> {
    let services = self.personal_services.iter().map(|name| Step::Call { space: personal.to_string(), actor: self.actor(), key: "POST [channel]/services", input: json!({"name": name}) });
    let seal = Step::Call { space: self.channel.clone(), actor: self.actor(), key: "POST [channel]/seal", input: json!({}) };
    services.chain(std::iter::once(seal)).collect()
  }

  fn identity(&self, personal: &str) -> Identity {
    Identity { slug: self.slug.clone(), id: self.id.clone(), channel: self.channel.clone(), personal: personal.to_string(), device: self.keys.public.clone() }
  }
}
