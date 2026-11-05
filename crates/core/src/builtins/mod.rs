//! The replicated services CarbonDB itself runs: `[space]` in every space, and
//! `[identity]` and `[device]` where a space's origin names them. Each one is
//! its migrations and an entry point that routes its own calls; CarbonDB routes
//! to it by service name only. `[@carbon]`, the device's local service, is host
//! code and never replicates.

pub mod carbon;
pub mod device;
pub mod identity;
pub mod space;

use serde_json::Value;

use crate::call::{Actor, Call, Outcome};
use crate::error::{Result, not_found};
use crate::store::{Side, Store};

// A builtin service: its name, its migrations in order, and its entry point.
pub struct Builtin {
  pub name: &'static str,
  pub migrations: &'static [&'static str],
  pub handle: fn(&Store, &Actor, &Call, Value) -> Result<Outcome>,
}

pub const BUILTINS: [Builtin; 3] = [
  Builtin { name: "space", migrations: space::MIGRATIONS, handle: space::handle },
  Builtin { name: "identity", migrations: identity::MIGRATIONS, handle: identity::handle },
  Builtin { name: "device", migrations: device::MIGRATIONS, handle: device::handle },
];

fn builtin(name: &str) -> Option<&'static Builtin> {
  BUILTINS.iter().find(|builtin| builtin.name == name)
}

// Run the migrations of `[space]` and of each builtin among a space's services.
pub fn migrate(store: &Store, services: &[String]) -> Result<()> {
  let named = std::iter::once("space").chain(services.iter().map(String::as_str));
  for builtin in named.filter_map(builtin) {
    crate::space::migrate(store, builtin.name, builtin.migrations)?;
  }
  Ok(())
}

// A call to a replicated builtin, routed by its service name.
pub fn handle(store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  let service = builtin(&call.service).ok_or_else(|| call.unknown())?;
  require_in_scope(store, call)?;
  (service.handle)(store, actor, call, input)
}

// `[space]` runs everywhere; `[identity]` and `[device]` only where the origin names them.
fn require_in_scope(store: &Store, call: &Call) -> Result<()> {
  if call.service == "space" || crate::space::origin(store, Side::Reader)?.services.contains(&call.service) {
    Ok(())
  } else {
    Err(not_found(format!("[{}] does not run in this space", call.service)))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  use crate::space::Origin;
  use crate::testing::call as key;

  fn alice() -> Actor {
    Actor { identity: Some("alice".into()), device: Some("pixel".into()) }
  }

  fn personal() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let origin = Origin { kind: "personal".into(), name: "alice".into(), services: vec!["device".into(), "chat".into()], manage: vec!["alice".into()] };
    let store = crate::space::create(&directory.path().join("p.db"), &origin, |_| Ok(())).unwrap();
    (directory, store)
  }

  #[test]
  fn a_manager_writes_and_reads_refs() {
    let (_directory, store) = personal();
    let reference = json!({"space": "s1", "kind": "company", "name": "FooCorp", "slug": "foocorp"});
    let wrote = handle(&store, &alice(), &key("POST [space]/ref"), reference).unwrap();
    assert_eq!((wrote.wrote, wrote.tables), (true, vec!["carbon_ref".to_string()]));
    let read = handle(&store, &alice(), &key("GET [space]/refs"), Value::Null).unwrap();
    assert_eq!(read.value, json!([{"space": "s1", "kind": "company", "name": "FooCorp", "slug": "foocorp", "cascade": []}]));
    assert_eq!((read.wrote, read.tables), (false, vec!["carbon_ref".to_string()]));
  }

  #[test]
  fn a_device_ref_replaces_the_last_one_for_its_key() {
    let (_directory, store) = personal();
    handle(&store, &alice(), &key("POST [device]/ref"), json!({"pubkey": "k1", "name": "Pixel9", "tags": ["manage"]})).unwrap();
    handle(&store, &alice(), &key("POST [device]/ref"), json!({"pubkey": "k1", "name": "Pixel9", "tags": []})).unwrap();
    let read = handle(&store, &alice(), &key("GET [device]/refs"), Value::Null).unwrap();
    assert_eq!(read.value, json!([{"pubkey": "k1", "name": "Pixel9", "tags": []}]));
  }

  #[test]
  fn refuses_what_it_should() {
    let (_directory, store) = personal();
    let sally = Actor { identity: Some("sally".into()), device: None };
    let cases = [
      (&sally, "POST [space]/ref", json!({"space": "s1", "kind": "team", "name": "Sales", "slug": "sales"}), 403, "FORBIDDEN"),
      (&alice(), "POST [space]/origin", json!({"kind": "team", "name": "x", "services": [], "manage": []}), 409, "CONFLICT"),
      (&alice(), "POST [space]/members", json!({"identity": "sally"}), 501, "SINGLE_NODE"),
      (&alice(), "POST [space]/manage", json!({}), 501, "NOT_BUILT"),
      (&alice(), "POST [device]/origin", json!({"pubkey": "k2"}), 501, "SINGLE_NODE"),
      (&alice(), "GET [identity]/origin", Value::Null, 404, "NOT_FOUND"),
      (&alice(), "GET [space]/nothing", Value::Null, 404, "NOT_FOUND"),
      (&alice(), "GET [chat]/messages", Value::Null, 404, "NOT_FOUND"),
      (&alice(), "POST [space]/ref", json!({"space": "s1"}), 400, "BAD_REQUEST"),
    ];
    for (actor, key_text, input, status, code) in cases {
      let error = handle(&store, actor, &key(key_text), input).unwrap_err();
      assert_eq!((error.status, error.code.as_str()), (status, code), "{key_text}");
    }
  }
}
