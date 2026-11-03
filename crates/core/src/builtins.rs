//! The replicated builtin services: `[space]` in every space, and `[identity]`
//! and `[device]` where a space's origin names them. Every POST to them needs
//! manage on the space.

use serde_json::{Value, json};

use crate::call::{Actor, Call, Outcome};
use crate::error::{Result, not_built, not_found, single_node};
use crate::space::{self, DeviceRef, Origin, Ref};
use crate::store::{Side, Store};

pub fn call(store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  require_in_scope(store, call)?;
  let manager = actor.identity.as_deref();
  match (call.service.as_str(), call.method(), call.resource.as_str()) {
    ("space", "POST", "/origin") => Outcome::write(store, |store| {
      let origin: Origin = serde_json::from_value(input)?;
      space::write_origin(store, &origin)?;
      Ok(json!(origin))
    }),
    ("space", "POST", "/ref") => Outcome::write(store, |store| {
      space::require_manage(store, manager)?;
      let reference: Ref = serde_json::from_value(input)?;
      space::add_ref(store, &reference)?;
      Ok(json!(reference))
    }),
    ("space", "GET", "/refs") => Outcome::read(store, |store| Ok(json!(space::refs(store, Side::Reader)?))),
    ("space", "POST", "/members") => Err(single_node("Adding a member")),
    ("space", "POST", "/manage") => Err(not_built("Changing a manage list")),
    ("identity", "GET", "/origin") => Outcome::read(store, |store| {
      let row = store.get(Side::Reader, "SELECT id, device FROM carbon_identity", &[])?;
      Ok(row)
    }),
    ("device", "POST", "/origin") => Err(single_node("Adding a device")),
    ("device", "GET", "/devices") => Outcome::read(store, |store| Ok(json!(space::devices(store, Side::Reader)?))),
    ("device", "POST", "/ref") => Outcome::write(store, |store| {
      space::require_manage(store, manager)?;
      let device: DeviceRef = serde_json::from_value(input)?;
      space::put_device_ref(store, &device)?;
      Ok(json!(device))
    }),
    ("device", "GET", "/refs") => Outcome::read(store, |store| Ok(json!(space::device_refs(store, Side::Reader)?))),
    _ => Err(call.unknown()),
  }
}

// `[space]` runs everywhere; `[identity]` and `[device]` only where the origin names them.
fn require_in_scope(store: &Store, call: &Call) -> Result<()> {
  if call.service == "space" || space::origin(store, Side::Reader)?.services.contains(&call.service) {
    Ok(())
  } else {
    Err(not_found(format!("[{}] does not run in this space", call.service)))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::call as key;

  fn brandon() -> Actor {
    Actor { identity: Some("brandon".into()), device: Some("pixel".into()) }
  }

  fn personal() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let origin = Origin { kind: "personal".into(), name: "brandon".into(), services: vec!["device".into(), "chat".into()], manage: vec!["brandon".into()] };
    let store = space::create(&directory.path().join("p.db"), &origin, |_| Ok(())).unwrap();
    (directory, store)
  }

  #[test]
  fn a_manager_writes_and_reads_refs() {
    let (_directory, store) = personal();
    let reference = json!({"space": "s1", "kind": "company", "name": "FooCorp", "slug": "foocorp"});
    let wrote = call(&store, &brandon(), &key("POST [space]/ref"), reference).unwrap();
    assert_eq!((wrote.wrote, wrote.tables), (true, vec!["carbon_ref".to_string()]));
    let read = call(&store, &brandon(), &key("GET [space]/refs"), Value::Null).unwrap();
    assert_eq!(read.value, json!([{"space": "s1", "kind": "company", "name": "FooCorp", "slug": "foocorp", "cascade": []}]));
    assert_eq!((read.wrote, read.tables), (false, vec!["carbon_ref".to_string()]));
  }

  #[test]
  fn a_device_ref_replaces_the_last_one_for_its_key() {
    let (_directory, store) = personal();
    call(&store, &brandon(), &key("POST [device]/ref"), json!({"pubkey": "k1", "name": "Pixel9", "tags": ["manage"]})).unwrap();
    call(&store, &brandon(), &key("POST [device]/ref"), json!({"pubkey": "k1", "name": "Pixel9", "tags": []})).unwrap();
    let read = call(&store, &brandon(), &key("GET [device]/refs"), Value::Null).unwrap();
    assert_eq!(read.value, json!([{"pubkey": "k1", "name": "Pixel9", "tags": []}]));
  }

  #[test]
  fn refuses_what_it_should() {
    let (_directory, store) = personal();
    let sally = Actor { identity: Some("sally".into()), device: None };
    let cases = [
      (&sally, "POST [space]/ref", json!({"space": "s1", "kind": "team", "name": "Sales", "slug": "sales"}), 403, "FORBIDDEN"),
      (&brandon(), "POST [space]/origin", json!({"kind": "team", "name": "x", "services": [], "manage": []}), 409, "CONFLICT"),
      (&brandon(), "POST [space]/members", json!({"identity": "sally"}), 501, "SINGLE_NODE"),
      (&brandon(), "POST [device]/origin", json!({"pubkey": "k2"}), 501, "SINGLE_NODE"),
      (&brandon(), "GET [identity]/origin", Value::Null, 404, "NOT_FOUND"),
      (&brandon(), "GET [space]/nothing", Value::Null, 404, "NOT_FOUND"),
      (&brandon(), "POST [space]/ref", json!({"space": "s1"}), 400, "BAD_REQUEST"),
    ];
    for (actor, key_text, input, status, code) in cases {
      let error = call(&store, actor, &key(key_text), input).unwrap_err();
      assert_eq!((error.status, error.code.as_str()), (status, code), "{key_text}");
    }
  }
}
