//! Creating identities and spaces: what `POST [@carbon]/identity` and
//! `POST [@carbon]/space` bring into being. `Carbon` routes those calls here
//! and keeps the system database; this module decides what each new space
//! holds and how it is named and attached.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::call::{Actor, Call, Outcome};
use crate::carbon::{Carbon, Identity};
use crate::error::{Result, bad_request, conflict};
use crate::hash::hash;
use crate::keys;
use crate::names::{slugify, unique_slug};
use crate::space::{self, DeviceRef, Origin, Ref};
use crate::store::{Side, Store};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityInput {
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

// Generate the first device's key pair, and create the identity space and the
// personal space, with a ref from the first to the second. Returns the identity
// and the device's secret key, which only the system database may keep.
pub fn identity(carbon: &Carbon, input: IdentityInput) -> Result<(Identity, String)> {
  if slugify(&input.slug) != input.slug || input.slug.is_empty() {
    return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", input.slug)));
  }
  if carbon.identity(&input.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", input.slug)));
  }
  let device = keys::generate()?;
  let id = hash(&json!({"key": "POST [identity]/origin", "input": {"device": device.public}}));
  let op = carbon.stamp(&json!({"key": "POST [@carbon]/identity", "input": {"slug": input.slug, "device": input.device, "services": input.services}}));
  let identity_space = hash(&json!({"op": op.hash, "space": "identity"}));
  let personal = hash(&json!({"op": op.hash, "space": "personal"}));

  let identity_origin = Origin { kind: "identity".into(), name: input.slug.clone(), services: vec!["identity".into(), "device".into()], manage: vec!["self".into()] };
  let personal_ref = Ref { space: personal.clone(), kind: "personal".into(), name: input.slug.clone(), slug: input.slug.clone(), cascade: vec![] };
  space::create(&carbon.space_path(&identity_space), &identity_origin, |store| {
    space::write_identity_origin(store, &id, &device.public)?;
    space::add_device(store, &device.public)?;
    space::add_ref(store, &personal_ref)
  })?;

  let personal_services = [vec!["device".to_string()], input.services.into_iter().filter(|service| service != "device").collect()].concat();
  let personal_origin = Origin { kind: "personal".into(), name: input.slug.clone(), services: personal_services, manage: vec![id.clone()] };
  let first_device = DeviceRef { pubkey: device.public.clone(), name: input.device, tags: vec!["manage".into()] };
  space::create(&carbon.space_path(&personal), &personal_origin, |store| space::put_device_ref(store, &first_device))?;

  Ok((Identity { slug: input.slug, id, space: identity_space, personal, device: device.public }, device.secret))
}

// Create a space nested in the calling one: the caller must manage it, since
// the new space is attached by a ref written there. The id is the op's hash.
pub fn nested_space(carbon: &Carbon, parent: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  let author = actor.identity.clone().ok_or_else(|| bad_request("Creating a space needs an acting identity"))?;
  let request: SpaceInput = serde_json::from_value(input.clone())?;
  let base = request.slug.clone().unwrap_or_else(|| slugify(&request.name));
  if base.is_empty() || slugify(&base) != base {
    return Err(bad_request(format!("'{}' gives no usable slug", request.name)));
  }
  let id = carbon.stamp(&json!({"key": call.key(), "input": input, "author": author})).hash;
  let origin = Origin { kind: request.kind.clone(), name: request.name.clone(), services: request.services, manage: request.manage.unwrap_or_else(|| vec![author.clone()]) };

  Outcome::write(parent, |parent| {
    space::require_manage(parent, Some(&author))?;
    let slug = unique_slug(&base, |slug| space::find_ref(parent, Side::Writer, &request.kind, slug).ok().flatten().is_some());
    space::create(&carbon.space_path(&id), &origin, |_| Ok(()))?;
    let reference = Ref { space: id.clone(), kind: request.kind.clone(), name: request.name.clone(), slug, cascade: request.cascade.clone() };
    space::add_ref(parent, &reference)?;
    Ok(json!(reference))
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{actor, brandon, call, carbon};

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
    let input = IdentityInput { slug: "brandon".into(), device: "Pixel9".into(), services: vec![] };
    let (identity, secret) = super::identity(&carbon, input).unwrap();
    assert_eq!(identity.id, hash(&json!({"key": "POST [identity]/origin", "input": {"device": identity.device}})));
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
}
