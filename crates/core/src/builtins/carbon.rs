//! `[@carbon]`, the device's local service: host code that never replicates,
//! so no peer checks its version. It answers on the system handle, where
//! identities are created and listed, and in every space, where spaces are
//! created and the space's status is read. It routes its own calls; the setup
//! it asks for lives in `identity` and `space`. Calls that need peers, or that
//! are designed but not built, are refused here.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::call::{Actor, Call, Outcome};
use crate::carbon::{Carbon, Context};
use crate::error::{Result, bad_request, conflict, not_built, single_node};
use crate::identity::{self, NewIdentity};
use crate::names::{slugify, unique_slug};
use crate::space::{self, Origin, Ref};
use crate::store::{Side, Store};

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

pub fn handle(carbon: &Carbon, context: Context, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  match (context, call.method(), call.resource.as_str()) {
    (Context::System, "POST", "/identity") => create_identity(carbon, call, input),
    (Context::System, "GET", "/identities") => Outcome::read(carbon.system(), |system| Ok(json!(identity::list(system, Side::Reader)?))),
    (Context::System, "POST", "/devices") => Err(single_node("Adding a device")),
    (Context::Space(store), "GET", "/status") => Outcome::read(store, |store| status(store, actor)),
    (Context::Space(store), "POST", "/space") => create_space(carbon, store, actor, call, input),
    (Context::Space(store), "GET", "/manage") => Outcome::read(store, |store| Ok(json!(space::managers(store, Side::Reader)?))),
    (Context::Space(_), _, "/members" | "/join" | "/sync/start" | "/sync/stop") => Err(single_node(&call.key())),
    (Context::Space(_), "POST", "/manage") => Err(not_built("Changing a manage list")),
    (Context::Space(_), "DELETE", "/data") => Err(not_built("Deleting a space's data")),
    _ => Err(call.unknown()),
  }
}

// Create an identity on this device. The system database records it in the
// same write, so an identity is listed only once both its spaces exist.
fn create_identity(carbon: &Carbon, call: &Call, input: Value) -> Result<Outcome> {
  let request: IdentityInput = serde_json::from_value(input)?;
  if request.slug.is_empty() || slugify(&request.slug) != request.slug {
    return Err(bad_request(format!("'{}' is not a lowercase kebab-case slug", request.slug)));
  }
  if carbon.identity(&request.slug).is_ok() {
    return Err(conflict(format!("An identity '{}' is already on this device", request.slug)));
  }
  let new = NewIdentity { slug: request.slug, device: request.device, services: request.services };
  let op = carbon.stamp(&json!({"key": call.key(), "input": {"slug": new.slug, "device": new.device, "services": new.services}})).hash;
  Outcome::write(carbon.system(), |_| Ok(json!(identity::create(carbon, &op, &new)?)))
}

// Create a space nested in the calling one: the caller must manage it, since
// the new space is attached by a ref written there. The id is the op's hash.
fn create_space(carbon: &Carbon, parent: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
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
    let reference = Ref { space: id.clone(), kind: request.kind, name: request.name, slug, cascade: request.cascade };
    space::add_ref(parent, &reference)?;
    Ok(json!(reference))
  })
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
  use crate::testing::{actor, alice, call, carbon};

  #[test]
  fn refuses_a_second_identity_with_the_same_slug_and_malformed_slugs() {
    let (_directory, carbon) = carbon();
    alice(&carbon);
    let cases = [("alice", 409), ("Alice", 400), ("", 400)];
    for (slug, status) in cases {
      let error = carbon.call(Context::System, &Actor::default(), &call("POST [@carbon]/identity"), json!({"slug": slug, "device": "x"})).unwrap_err();
      assert_eq!(error.status, status, "{slug}");
    }
  }

  #[test]
  fn creates_a_space_nested_in_the_calling_one() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    let create = |name: &str| carbon.call(Context::Space(&personal), &actor(&identity), &call("POST [@carbon]/space"), json!({"name": name, "services": ["chat"]}));

    let created = create("Randall's Playhouse").unwrap();
    assert_eq!((created.value["slug"].as_str(), created.value["kind"].as_str()), (Some("randalls-playhouse"), Some("space")));
    let again = create("Randall's Playhouse").unwrap();
    assert_eq!(again.value["slug"].as_str(), Some("randalls-playhouse-2"));

    let room = carbon.open_space(created.value["space"].as_str().unwrap()).unwrap();
    let status = carbon.call(Context::Space(&room), &actor(&identity), &call("GET [@carbon]/status"), Value::Null).unwrap();
    assert_eq!(status.value["name"], json!("Randall's Playhouse"));
    assert_eq!(status.value["services"], json!(["chat"]));
    assert_eq!(status.value["manage"], json!([identity.id]));
    assert_eq!((status.value["access"].clone(), status.value["sync"].clone(), status.value["peers"].clone()), (json!(["manage"]), json!("off"), json!([])));
  }

  #[test]
  fn only_a_manager_may_create_a_space_in_another() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    let stranger = Actor { identity: Some("someone-else".into()), device: None };
    let error = carbon.call(Context::Space(&personal), &stranger, &call("POST [@carbon]/space"), json!({"name": "Room"})).unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(space::refs(&personal, Side::Reader).unwrap(), vec![]);
  }

  #[test]
  fn refuses_calls_that_need_peers_or_are_not_built() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.open_space(&identity.personal).unwrap();
    let cases = [
      ("POST [@carbon]/members", "SINGLE_NODE"),
      ("POST [@carbon]/join", "SINGLE_NODE"),
      ("POST [@carbon]/sync/start", "SINGLE_NODE"),
      ("POST [@carbon]/manage", "NOT_BUILT"),
      ("DELETE [@carbon]/data", "NOT_BUILT"),
    ];
    for (key, code) in cases {
      let error = carbon.call(Context::Space(&personal), &actor(&identity), &call(key), json!({})).unwrap_err();
      assert_eq!((error.status, error.code.as_str()), (501, code), "{key}");
    }
    assert_eq!(carbon.call(Context::System, &Actor::default(), &call("POST [@carbon]/devices"), json!({})).unwrap_err().code, "SINGLE_NODE");
  }
}
