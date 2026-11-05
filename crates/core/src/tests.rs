//! Tests across the core: how calls apply, nest and roll back; what the op log
//! records; and what `[@carbon]` sets up, through the builtins, as ops.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::call::Actor;
use crate::oplog;
use crate::store::Db;
use crate::testing::{actor, alice, call_now, carbon, room, texts};

// The keys of a space's op log, in order.
fn logged(space: &crate::Space) -> Vec<String> {
  oplog::entries(space.store()).unwrap().into_iter().map(|entry| entry.key).collect()
}

mod transactions {
  use super::*;

  // A script for `POST [notes]/run`.
  fn run(script: Value) -> Value {
    json!({"do": script})
  }

  #[test]
  fn calls_inside_an_op_see_its_writes_and_roll_back_on_their_own() {
    let cases = [
      ("a call inside an op sees the op's writes", json!([{"insert": "a"}, {"call": "GET [notes]/texts"}]), Ok(json!([["a"]])), json!(["a"])),
      (
        "a failing call the handler catches undoes only its own writes",
        json!([{"insert": "a"}, {"call": "POST [notes]/run", "input": run(json!([{"insert": "b"}, {"fail": "no"}])), "catch": true}]),
        Ok(json!([{"error": 409}])),
        json!(["a"]),
      ),
      (
        "a failing call the handler does not catch fails the op",
        json!([{"insert": "a"}, {"call": "POST [notes]/run", "input": run(json!([{"insert": "b"}, {"fail": "no"}]))}]),
        Err(409),
        json!([]),
      ),
      (
        "an op failing after its calls undoes theirs too",
        json!([{"call": "POST [notes]/run", "input": run(json!([{"insert": "b"}]))}, {"fail": "no"}]),
        Err(409),
        json!([]),
      ),
      (
        "calls nest, each seeing every write around it",
        json!([{"insert": "a"}, {"call": "POST [notes]/run", "input": run(json!([{"insert": "b"}, {"read": true}]))}, {"read": true}]),
        Ok(json!([[["a", "b"]], ["a", "b"]])),
        json!(["a", "b"]),
      ),
      (
        "a nested failure undoes only the innermost call",
        json!([{"insert": "a"}, {"call": "POST [notes]/run", "input": run(json!([{"insert": "b"}, {"call": "POST [notes]/run", "input": run(json!([{"insert": "c"}, {"fail": "no"}])), "catch": true}]))}]),
        Ok(json!([[{"error": 409}]])),
        json!(["a", "b"]),
      ),
      ("a replicated service never calls a local one", json!([{"call": "GET [@carbon]/status"}]), Err(403), json!([])),
    ];
    for (why, script, expected, committed) in cases {
      let (_directory, carbon) = carbon();
      let identity = alice(&carbon);
      let space = room(&carbon, &identity);
      let result = call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", run(script));
      assert_eq!(result.map(|settled| settled.value).map_err(|error| error.status), expected, "{why}");
      assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), committed, "{why}");
    }
  }

  #[test]
  fn a_get_reads_and_calls_reads_only() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", run(json!([{"insert": "a"}]))).unwrap();
    let cases = [
      ("a GET reads", json!([{"read": true}]), Ok(json!([["a"]]))),
      ("a GET calls another GET", json!([{"call": "GET [notes]/texts"}]), Ok(json!([["a"]]))),
      ("a GET never writes", json!([{"insert": "b"}]), Err(400)),
      ("a GET never calls a write", json!([{"call": "POST [notes]/run", "input": run(json!([]))}]), Err(400)),
    ];
    for (why, script, expected) in cases {
      let result = call_now(&carbon, &space, &actor(&identity), "GET [notes]/run", run(script));
      assert_eq!(result.map(|settled| settled.value).map_err(|error| error.status), expected, "{why}");
    }
  }

  #[test]
  fn a_commit_reports_the_tables_it_wrote_and_a_get_those_it_read() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let wrote = call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", run(json!([{"insert": "a"}]))).unwrap();
    assert_eq!(wrote.tables, vec!["carbon_oplog", "note"]);
    let read = call_now(&carbon, &space, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();
    assert_eq!(read.tables, vec!["note"]);
  }
}

mod oplog_entries {
  use super::*;

  #[test]
  fn logs_an_applied_op_once_with_its_output_and_calls_and_nothing_else() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let script = json!({"do": [{"call": "POST [notes]/run", "input": {"do": [{"insert": "a"}]}}]});
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", script.clone()).unwrap();
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", json!({"do": [{"fail": "no"}]})).unwrap_err();
    call_now(&carbon, &space, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();

    let entries = oplog::entries(space.store()).unwrap();
    let applied: Vec<_> = entries.iter().filter(|entry| entry.key == "POST [notes]/run").collect();
    assert_eq!(applied.len(), 1);
    let entry = applied[0];
    assert_eq!((&entry.author, &entry.device, &entry.input, entry.version), (&Some(identity.id.clone()), &Some(identity.device.clone()), &script, 1));
    assert_eq!(entry.output, json!([[]]));
    assert_eq!(entry.calls, json!([{"key": "POST [notes]/run", "input": {"do": [{"insert": "a"}]}, "output": []}]));
  }

  #[test]
  fn keeps_the_log_and_data_across_a_restart() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", json!({"do": [{"insert": "kept"}]})).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::Carbon::open(&directory.path().join("carbondb.toml")).unwrap();
    crate::testing::register_notes(&restarted);
    let space = restarted.space(&id).unwrap();
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["kept"]));
    assert_eq!(logged(&space), vec!["POST [space]/origin", "POST [notes]/run"]);
  }
}

mod identities {
  use super::*;
  use crate::builtins::{device, identity as identity_service, space as space_service};
  use crate::identity::{id_of, list};

  #[test]
  fn sets_up_both_spaces_by_ops_and_logs_its_own_call_in_the_system_log() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(carbon.identity("alice").unwrap(), identity);
    assert_eq!(identity.id, id_of(&identity.device));
    assert_eq!(logged(carbon.system()), vec!["POST [@carbon]/identity"]);

    let identity_space = carbon.space(&identity.space).unwrap();
    assert_eq!(logged(&identity_space), vec!["POST [space]/origin", "POST [identity]/origin", "POST [device]/origin", "POST [space]/ref"]);
    let authors: Vec<_> = oplog::entries(identity_space.store()).unwrap().into_iter().map(|entry| (entry.author, entry.device)).collect();
    assert!(authors.iter().all(|author| *author == (Some(identity.id.clone()), Some(identity.device.clone()))));
    let reader = identity_space.store().reader().unwrap();
    assert_eq!(space_service::managers(&reader).unwrap(), vec![identity.id.clone()]);
    assert_eq!(identity_service::id(&reader).unwrap(), Some(identity.id.clone()));
    assert_eq!(device::devices(&reader).unwrap(), vec![identity.device.clone()]);
    assert_eq!(space_service::find_ref(&reader, "personal", "alice").unwrap().unwrap().space, identity.personal);

    let personal = carbon.space(&identity.personal).unwrap();
    assert_eq!(logged(&personal), vec!["POST [space]/origin", "POST [device]/ref"]);
    let reader = personal.store().reader().unwrap();
    let origin = space_service::origin(&reader).unwrap();
    assert_eq!((origin.kind.as_str(), origin.services, origin.manage), ("personal", vec!["device".to_string(), "notes".to_string()], vec![identity.id.clone()]));
    assert_eq!(device::device_refs(&reader).unwrap(), vec![device::DeviceRef { pubkey: identity.device.clone(), name: "Pixel9".into(), tags: vec!["manage".into()] }]);
  }

  #[test]
  fn only_the_system_database_keeps_the_secret() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let row = carbon.system().store().reader().unwrap().get("SELECT secret FROM carbon_system_identity", &[]).unwrap();
    let secret = row["secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);
    for id in [&identity.space, &identity.personal] {
      let bytes = std::fs::read(directory.path().join("data").join(format!("{id}.db"))).unwrap();
      assert!(!String::from_utf8_lossy(&bytes).contains(&secret));
    }
    assert!(list(&carbon.system().store().reader().unwrap()).unwrap()[0].get("secret").is_none());
    let logged = serde_json::to_string(&oplog::entries(carbon.system().store()).unwrap()[0].output).unwrap();
    assert!(!logged.contains(&secret));
  }

  #[test]
  fn refuses_a_taken_or_malformed_slug_and_unregistered_services() {
    let (_directory, carbon) = carbon();
    alice(&carbon);
    let cases = [
      (json!({"slug": "alice", "device": "x"}), 409),
      (json!({"slug": "Alice", "device": "x"}), 400),
      (json!({"slug": "", "device": "x"}), 400),
      (json!({"slug": "sally", "device": "x", "services": ["crm"]}), 400),
    ];
    for (input, status) in cases {
      let error = call_now(&carbon, carbon.system(), &Actor::default(), "POST [@carbon]/identity", input.clone()).unwrap_err();
      assert_eq!(error.status, status, "{input}");
    }
    assert_eq!(logged(carbon.system()).len(), 1);
  }
}

mod spaces {
  use super::*;
  use crate::builtins::space as space_service;

  #[test]
  fn creates_a_space_by_a_ref_op_in_the_calling_space_and_an_origin_op_in_the_new_one() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.space(&identity.personal).unwrap();
    let create = |name: &str| call_now(&carbon, &personal, &actor(&identity), "POST [@carbon]/space", json!({"name": name, "services": ["notes"]})).unwrap().value;

    let created = create("Randall's Playhouse");
    assert_eq!((created["slug"].as_str(), created["kind"].as_str()), (Some("randalls-playhouse"), Some("space")));
    assert_eq!(create("Randall's Playhouse")["slug"].as_str(), Some("randalls-playhouse-2"));

    let entries = oplog::entries(personal.store()).unwrap();
    let keys: Vec<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(keys[2..4], ["POST [@carbon]/space", "POST [space]/ref"]);
    let (local, reference) = (&entries[2], &entries[3]);
    assert_eq!(local.device, Some(identity.device.clone()));
    assert_eq!(local.calls[0]["op"], json!(reference.hash));
    assert_eq!(local.hash, created["space"].as_str().unwrap());

    let room = carbon.space(created["space"].as_str().unwrap()).unwrap();
    assert_eq!(logged(&room), vec!["POST [space]/origin"]);
    let status = call_now(&carbon, &room, &actor(&identity), "GET [@carbon]/status", Value::Null).unwrap().value;
    assert_eq!(
      (&status["name"], &status["services"], &status["manage"], &status["access"], &status["sync"], &status["peers"]),
      (&json!("Randall's Playhouse"), &json!(["notes"]), &json!([identity.id]), &json!(["manage"]), &json!("off"), &json!([]))
    );
  }

  #[test]
  fn only_a_manager_may_create_a_space_in_another() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.space(&identity.personal).unwrap();
    let stranger = Actor { identity: Some("someone-else".into()), device: None };
    let error = call_now(&carbon, &personal, &stranger, "POST [@carbon]/space", json!({"name": "Room"})).unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(space_service::refs(&personal.store().reader().unwrap()).unwrap(), vec![]);
    assert_eq!(logged(&personal), vec!["POST [space]/origin", "POST [device]/ref"]);
  }

  #[test]
  fn refuses_calls_that_need_peers_are_not_built_or_break_the_rules() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.space(&identity.personal).unwrap();
    let identity_space = carbon.space(&identity.space).unwrap();
    let alice = actor(&identity);
    let sally = Actor { identity: Some("sally".into()), device: None };
    let system = Rc::clone(carbon.system());
    let cases = [
      (&personal, &alice, "POST [@carbon]/members", json!({}), "SINGLE_NODE"),
      (&personal, &alice, "POST [@carbon]/join", json!({}), "SINGLE_NODE"),
      (&personal, &alice, "POST [@carbon]/sync/start", json!({}), "SINGLE_NODE"),
      (&personal, &alice, "POST [@carbon]/manage", json!({}), "NOT_BUILT"),
      (&personal, &alice, "DELETE [@carbon]/data", json!({}), "NOT_BUILT"),
      (&system, &Actor::default(), "POST [@carbon]/devices", json!({}), "SINGLE_NODE"),
      (&personal, &alice, "POST [space]/members", json!({"identity": "sally"}), "SINGLE_NODE"),
      (&personal, &alice, "POST [space]/manage", json!({}), "NOT_BUILT"),
      (&identity_space, &alice, "POST [device]/origin", json!({"pubkey": "k2"}), "SINGLE_NODE"),
      (&personal, &alice, "POST [device]/origin", json!({"pubkey": "k2"}), "BAD_REQUEST"),
      (&personal, &sally, "POST [space]/ref", json!({"space": "s1", "kind": "team", "name": "Sales"}), "FORBIDDEN"),
      (&personal, &alice, "POST [space]/origin", json!({"kind": "team", "name": "x", "services": [], "manage": []}), "CONFLICT"),
      (&personal, &alice, "POST [space]/ref", json!({"space": "s1"}), "BAD_REQUEST"),
      (&personal, &alice, "POST [space]/ref", json!({"space": "s1", "kind": "team", "name": "!!"}), "BAD_REQUEST"),
      (&personal, &alice, "GET [space]/nothing", Value::Null, "NOT_FOUND"),
      (&personal, &alice, "GET [identity]/origin", Value::Null, "NOT_FOUND"),
    ];
    for (space, actor, key, input, code) in cases {
      let error = call_now(&carbon, space, actor, key, input).unwrap_err();
      assert_eq!(error.code, code, "{key}");
    }
  }

  #[test]
  fn a_ref_to_the_same_space_replaces_the_last_and_a_device_ref_the_last_for_its_key() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = carbon.space(&identity.personal).unwrap();
    let alice = actor(&identity);
    for name in ["Sales", "Sales West"] {
      call_now(&carbon, &personal, &alice, "POST [space]/ref", json!({"space": "s1", "kind": "team", "name": name})).unwrap();
    }
    let refs = call_now(&carbon, &personal, &alice, "GET [space]/refs", Value::Null).unwrap().value;
    assert_eq!(refs, json!([{"space": "s1", "kind": "team", "name": "Sales West", "slug": "sales-west", "cascade": []}]));

    call_now(&carbon, &personal, &alice, "POST [device]/ref", json!({"pubkey": identity.device, "name": "Pixel9", "tags": []})).unwrap();
    let devices = call_now(&carbon, &personal, &alice, "GET [device]/refs", Value::Null).unwrap().value;
    assert_eq!(devices, json!([{"pubkey": identity.device, "name": "Pixel9", "tags": []}]));
  }
}
