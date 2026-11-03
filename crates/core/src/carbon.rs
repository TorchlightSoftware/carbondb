//! `[@carbon]`, the core's local service, and the system database it keeps.
//!
//! The system database is this device's root: the identities on it, with their
//! device keys. Every other space is a file beside it, named by space id.
//!
//! This is the single-node core: no p2panda, no network and no op log. Calls
//! that need peers are refused.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::builtins;
use crate::call::{Actor, Call, Outcome};
use crate::clock::Clock;
use crate::config::{Config, SyncMode};
use crate::error::{Result, bad_request, conflict, not_built, not_found, single_node};
use crate::hash::hash;
use crate::keys;
use crate::names::{slugify, unique_slug};
use crate::space::{self, DeviceRef, Origin, Ref};
use crate::store::{Side, Store};

const SYSTEM_SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS carbon_system_identity (
    slug TEXT PRIMARY KEY, id TEXT NOT NULL UNIQUE, space TEXT NOT NULL, personal TEXT NOT NULL,
    device TEXT NOT NULL, secret TEXT NOT NULL
  );
";

// An identity on this device, as anyone on it may see it: everything but the secret key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
  pub slug: String,
  pub id: String,
  pub space: String,
  pub personal: String,
  pub device: String,
}

// When an op happened, and its hash over the envelope and that time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stamp {
  pub time: u64,
  pub hash: String,
}

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

pub struct Carbon {
  config: Config,
  system: Store,
  clock: Clock,
}

impl Carbon {
  pub fn open(config_path: &Path) -> Result<Carbon> {
    let config = Config::load(config_path)?;
    if config.sync != SyncMode::None {
      return Err(not_built("Replicated mode"));
    }
    std::fs::create_dir_all(config.directory()).map_err(|error| bad_request(format!("Cannot create {}: {error}", config.directory().display())))?;
    let system = Store::open(&config.system)?;
    system.run_batch(SYSTEM_SCHEMA)?;
    Ok(Carbon { config, system, clock: Clock::default() })
  }

  // Stamp an op's envelope with a time, and hash the two together.
  pub fn stamp(&self, envelope: &Value) -> Stamp {
    let time = self.clock.now();
    Stamp { time, hash: hash(&json!({"envelope": envelope, "time": time})) }
  }

  fn space_path(&self, id: &str) -> PathBuf {
    self.config.directory().join(format!("{id}.db"))
  }

  pub fn open_space(&self, id: &str) -> Result<Store> {
    let path = self.space_path(id);
    if !path.exists() {
      return Err(not_found(format!("No space {id} on this device")));
    }
    Store::open(&path)
  }

  pub fn identity(&self, slug: &str) -> Result<Identity> {
    let row = self.system.get(Side::Reader, "SELECT slug, id, space, personal, device FROM carbon_system_identity WHERE slug = ?", &[json!(slug)])?;
    if row.is_null() {
      return Err(not_found(format!("No identity '{slug}' on this device")));
    }
    Ok(serde_json::from_value(row)?)
  }

  // A call on the system handle.
  pub fn call_system(&self, call: &Call, input: Value) -> Result<Outcome> {
    match (call.confined, call.service.as_str(), call.method(), call.resource.as_str()) {
      (true, "carbon", "POST", "/identity") => self.create_identity(serde_json::from_value(input)?),
      (true, "carbon", "GET", "/identities") => Outcome::read(&self.system, |system| {
        Ok(json!(system.all(Side::Reader, "SELECT slug, id, space, personal, device FROM carbon_system_identity ORDER BY slug", &[])?))
      }),
      (true, "carbon", "POST", "/devices") => Err(single_node("Adding a device")),
      _ => Err(call.unknown()),
    }
  }

  // A call on a space's handle, to `[@carbon]` or a replicated builtin.
  pub fn call_space(&self, store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
    if !call.confined {
      return builtins::call(store, actor, call, input);
    }
    match (call.service.as_str(), call.method(), call.resource.as_str()) {
      ("carbon", "GET", "/status") => Outcome::read(store, |store| status(store, actor)),
      ("carbon", "POST", "/space") => self.create_space(store, actor, call, input),
      ("carbon", "GET", "/manage") => Outcome::read(store, |store| Ok(json!(space::managers(store, Side::Reader)?))),
      ("carbon", _, "/members" | "/join" | "/sync/start" | "/sync/stop") => Err(single_node(&call.key())),
      ("carbon", "POST", "/manage") => Err(not_built("Changing a manage list")),
      ("carbon", "DELETE", "/data") => Err(not_built("Deleting a space's data")),
      _ => Err(call.unknown()),
    }
  }

  // Generate the first device's key pair, and create the identity space and the
  // personal space, with a ref from the first to the second.
  fn create_identity(&self, input: IdentityInput) -> Result<Outcome> {
    if slugify(&input.slug) != input.slug || input.slug.is_empty() {
      return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", input.slug)));
    }
    if self.identity(&input.slug).is_ok() {
      return Err(conflict(format!("An identity '{}' is already on this device", input.slug)));
    }
    let device = keys::generate()?;
    let id = hash(&json!({"key": "POST [identity]/origin", "input": {"device": device.public}}));
    let op = self.stamp(&json!({"key": "POST [@carbon]/identity", "input": {"slug": input.slug, "device": input.device, "services": input.services}}));
    let identity_space = hash(&json!({"op": op.hash, "space": "identity"}));
    let personal = hash(&json!({"op": op.hash, "space": "personal"}));

    let identity_origin = Origin { kind: "identity".into(), name: input.slug.clone(), services: vec!["identity".into(), "device".into()], manage: vec!["self".into()] };
    let personal_ref = Ref { space: personal.clone(), kind: "personal".into(), name: input.slug.clone(), slug: input.slug.clone(), cascade: vec![] };
    space::create(&self.space_path(&identity_space), &identity_origin, |store| {
      space::write_identity_origin(store, &id, &device.public)?;
      space::add_device(store, &device.public)?;
      space::add_ref(store, &personal_ref)
    })?;

    let personal_services = [vec!["device".to_string()], input.services.into_iter().filter(|service| service != "device").collect()].concat();
    let personal_origin = Origin { kind: "personal".into(), name: input.slug.clone(), services: personal_services, manage: vec![id.clone()] };
    let first_device = DeviceRef { pubkey: device.public.clone(), name: input.device, tags: vec!["manage".into()] };
    space::create(&self.space_path(&personal), &personal_origin, |store| space::put_device_ref(store, &first_device))?;

    let identity = Identity { slug: input.slug, id, space: identity_space, personal, device: device.public };
    Outcome::write(&self.system, |system| {
      system.run(
        Side::Writer,
        "INSERT INTO carbon_system_identity (slug, id, space, personal, device, secret) VALUES (?, ?, ?, ?, ?, ?)",
        &[json!(identity.slug), json!(identity.id), json!(identity.space), json!(identity.personal), json!(identity.device), json!(device.secret)],
      )?;
      Ok(json!(identity))
    })
  }

  // Create a space nested in the calling one: the caller must manage it, since
  // the new space is attached by a ref written there. The id is the op's hash.
  fn create_space(&self, parent: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
    let author = actor.identity.clone().ok_or_else(|| bad_request("Creating a space needs an acting identity"))?;
    let request: SpaceInput = serde_json::from_value(input.clone())?;
    let base = request.slug.clone().unwrap_or_else(|| slugify(&request.name));
    if base.is_empty() || slugify(&base) != base {
      return Err(bad_request(format!("'{}' gives no usable slug", request.name)));
    }
    let id = self.stamp(&json!({"key": call.key(), "input": input, "author": author})).hash;
    let origin = Origin { kind: request.kind.clone(), name: request.name.clone(), services: request.services, manage: request.manage.unwrap_or_else(|| vec![author.clone()]) };

    Outcome::write(parent, |parent| {
      space::require_manage(parent, Some(&author))?;
      let slug = unique_slug(&base, |slug| space::find_ref(parent, Side::Writer, &request.kind, slug).ok().flatten().is_some());
      space::create(&self.space_path(&id), &origin, |_| Ok(()))?;
      let reference = Ref { space: id.clone(), kind: request.kind.clone(), name: request.name.clone(), slug, cascade: request.cascade.clone() };
      space::add_ref(parent, &reference)?;
      Ok(json!(reference))
    })
  }
}

// What a space is, and its state on this device. Single-node: sync is off, there
// are no peers, and every identity here has full access.
fn status(store: &Store, actor: &Actor) -> Result<Value> {
  let origin = space::origin(store, Side::Reader)?;
  let managers = space::managers(store, Side::Reader)?;
  let manages = actor.identity.as_ref().is_some_and(|identity| managers.contains(identity));
  let id = store.path().file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
  Ok(json!({
    "space": id,
    "kind": origin.kind,
    "name": origin.name,
    "services": origin.services,
    "manage": origin.manage,
    "access": if manages { vec!["manage"] } else { vec!["write"] },
    "sync": "off",
    "peers": [],
  }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn call(key: &str) -> Call {
    let (method, rest) = key.split_once(' ').unwrap();
    let (service, resource) = rest.trim_start_matches('[').split_once(']').unwrap();
    let confined = service.starts_with('@');
    Call { method: Some(method.into()), service: service.trim_start_matches('@').into(), resource: resource.into(), confined }
  }

  fn carbon() -> (tempfile::TempDir, Carbon) {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("carbondb.toml");
    std::fs::write(&config, "system = \"data/system.db\"\n[sync]\nmode = \"none\"\n").unwrap();
    let carbon = Carbon::open(&config).unwrap();
    (directory, carbon)
  }

  fn brandon(carbon: &Carbon) -> Identity {
    let created = carbon.call_system(&call("POST [@carbon]/identity"), json!({"slug": "brandon", "device": "Pixel9", "services": ["chat"]})).unwrap();
    serde_json::from_value(created.value).unwrap()
  }

  fn actor(identity: &Identity) -> Actor {
    Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) }
  }

  #[test]
  fn refuses_replicated_mode_for_now() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("carbondb.toml");
    std::fs::write(&config, "system = \"system.db\"\n").unwrap();
    assert_eq!(Carbon::open(&config).err().unwrap().code, "NOT_BUILT");
  }

  #[test]
  fn creating_an_identity_creates_its_identity_and_personal_spaces() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    assert_eq!(carbon.identity("brandon").unwrap(), identity);

    let identity_space = carbon.open_space(&identity.space).unwrap();
    assert_eq!(space::identity_id(&identity_space, Side::Reader).unwrap(), Some(identity.id.clone()));
    assert_eq!(space::managers(&identity_space, Side::Reader).unwrap(), vec![identity.id.clone()]);
    assert_eq!(space::devices(&identity_space, Side::Reader).unwrap(), vec![identity.device.clone()]);
    let personal_ref = space::find_ref(&identity_space, Side::Reader, "personal", "brandon").unwrap().unwrap();
    assert_eq!(personal_ref.space, identity.personal);

    let personal = carbon.open_space(&identity.personal).unwrap();
    let origin = space::origin(&personal, Side::Reader).unwrap();
    assert_eq!((origin.kind.as_str(), origin.services, origin.manage), ("personal", vec!["device".to_string(), "chat".to_string()], vec![identity.id.clone()]));
    let devices = space::device_refs(&personal, Side::Reader).unwrap();
    assert_eq!(devices, vec![DeviceRef { pubkey: identity.device.clone(), name: "Pixel9".into(), tags: vec!["manage".into()] }]);
  }

  #[test]
  fn the_identity_id_is_the_hash_of_its_origin_and_keeps_no_secret_in_any_space() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    assert_eq!(identity.id, hash(&json!({"key": "POST [identity]/origin", "input": {"device": identity.device}})));
    let secret = carbon.system.get(Side::Reader, "SELECT secret FROM carbon_system_identity", &[]).unwrap()["secret"].as_str().unwrap().to_string();
    for id in [&identity.space, &identity.personal] {
      let bytes = std::fs::read(carbon.space_path(id)).unwrap();
      assert!(!String::from_utf8_lossy(&bytes).contains(&secret));
    }
  }

  #[test]
  fn refuses_a_second_identity_with_the_same_slug_and_malformed_slugs() {
    let (_directory, carbon) = carbon();
    brandon(&carbon);
    let cases = [("brandon", 409), ("Brandon", 400), ("", 400)];
    for (slug, status) in cases {
      let error = carbon.call_system(&call("POST [@carbon]/identity"), json!({"slug": slug, "device": "x"})).unwrap_err();
      assert_eq!(error.status, status, "{slug}");
    }
  }

  #[test]
  fn creates_a_space_nested_in_the_calling_one() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    let create = |name: &str| carbon.call_space(&personal, &actor(&identity), &call("POST [@carbon]/space"), json!({"name": name, "services": ["chat"]}));

    let created = create("Randall's Playhouse").unwrap();
    assert_eq!((created.value["slug"].as_str(), created.value["kind"].as_str()), (Some("randalls-playhouse"), Some("space")));
    let again = create("Randall's Playhouse").unwrap();
    assert_eq!(again.value["slug"].as_str(), Some("randalls-playhouse-2"));

    let room = carbon.open_space(created.value["space"].as_str().unwrap()).unwrap();
    let status = carbon.call_space(&room, &actor(&identity), &call("GET [@carbon]/status"), Value::Null).unwrap();
    assert_eq!(status.value["name"], json!("Randall's Playhouse"));
    assert_eq!(status.value["services"], json!(["chat"]));
    assert_eq!(status.value["manage"], json!([identity.id]));
    assert_eq!((status.value["access"].clone(), status.value["sync"].clone(), status.value["peers"].clone()), (json!(["manage"]), json!("off"), json!([])));
  }

  #[test]
  fn only_a_manager_may_create_a_space_in_another() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    let stranger = Actor { identity: Some("someone-else".into()), device: None };
    let error = carbon.call_space(&personal, &stranger, &call("POST [@carbon]/space"), json!({"name": "Room"})).unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(space::refs(&personal, Side::Reader).unwrap(), vec![]);
  }

  #[test]
  fn refuses_calls_that_need_peers() {
    let (_directory, carbon) = carbon();
    let identity = brandon(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    for key in ["POST [@carbon]/members", "POST [@carbon]/join", "POST [@carbon]/sync/start"] {
      let error = carbon.call_space(&personal, &actor(&identity), &call(key), json!({})).unwrap_err();
      assert_eq!((error.status, error.code.as_str()), (501, "SINGLE_NODE"), "{key}");
    }
    assert_eq!(carbon.call_system(&call("POST [@carbon]/devices"), json!({})).unwrap_err().code, "SINGLE_NODE");
  }
}
