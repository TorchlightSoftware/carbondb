//! Tests across the core: how calls apply, nest and roll back; what the op log
//! and results record; and what `[@carbon]` sets up, through the builtins, as ops.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::results::{self, Outcome};
use crate::testing::{actor, alice, call_now, carbon, room, space_now, texts};
use crate::{Actor, Db, Routing, oplog};

// The keys of a space's op log, in order.
fn logged(space: &crate::Space) -> Vec<String> {
  oplog::entries(space.store()).unwrap().into_iter().map(|op| op.key).collect()
}

// What became of each op in a space's log, in order: applied, or refused with its status.
fn outcomes(space: &crate::Space) -> Vec<Result<(), u16>> {
  let ops = oplog::entries(space.store()).unwrap();
  ops.iter().map(|op| match results::of(space.store(), &op.hash).unwrap() {
    Some(Outcome::Applied { .. }) => Ok(()),
    Some(Outcome::Refused { error, .. }) => Err(error.status),
    None => Err(0),
  }).collect()
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
    assert_eq!(wrote.tables, vec!["notes/db/note"]);
    let read = call_now(&carbon, &space, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();
    assert_eq!(read.tables, vec!["notes/db/note"]);
  }
}

mod oplog_entries {
  use super::*;

  #[test]
  fn logs_every_op_it_receives_and_keeps_what_became_of_it_as_its_result() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let script = json!({"do": [{"call": "POST [notes]/run", "input": {"do": [{"insert": "a"}]}}]});
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", script.clone()).unwrap();
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", json!({"do": [{"fail": "no"}]})).unwrap_err();
    call_now(&carbon, &space, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();

    assert_eq!(logged(&space), vec!["POST [space]/origin", "POST [notes]/run", "POST [notes]/run"]);
    assert_eq!(outcomes(&space), vec![Ok(()), Ok(()), Err(409)]);
    let applied = &oplog::entries(space.store()).unwrap()[1];
    assert_eq!((&applied.author, &applied.device, &applied.input, applied.version.as_str()), (&Some(identity.id.clone()), &Some(identity.device.clone()), &script, "notes-v1"));
    let calls = json!([{"key": "POST [notes]/run", "input": {"do": [{"insert": "a"}]}, "output": []}]);
    assert_eq!(results::of(space.store(), &applied.hash).unwrap(), Some(Outcome::Applied { output: json!([[]]), calls }));
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["a"]));
  }

  #[test]
  fn applies_an_op_left_without_a_result_when_its_space_opens_again() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let call = crate::Call::from_key("POST [notes]/run").unwrap();
    let op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"insert": "after a crash"}]}));
    oplog::append(space.store(), &op).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::Carbon::open(&directory.path().join("carbondb.toml")).unwrap();
    let space = space_now(&restarted, &id).unwrap();
    assert_eq!(outcomes(&space), vec![Ok(()), Err(0)]);
    crate::testing::register_notes(&restarted);
    assert_eq!(outcomes(&space), vec![Ok(()), Ok(())]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["after a crash"]));
  }

  #[test]
  fn refuses_an_op_made_for_another_version_of_its_service() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let call = crate::Call::from_key("POST [notes]/run").unwrap();
    let mut op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"insert": "old"}]}));
    op = crate::Op::new(op.key, op.input, op.author, op.device, "notes-v0".into(), op.time, op.hash);
    oplog::append(space.store(), &op).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::Carbon::open(&directory.path().join("carbondb.toml")).unwrap();
    crate::testing::register_notes(&restarted);
    let space = space_now(&restarted, &id).unwrap();
    assert_eq!(outcomes(&space), vec![Ok(()), Err(409)]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!([]));
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
    let space = space_now(&restarted, &id).unwrap();
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["kept"]));
    assert_eq!(logged(&space), vec!["POST [space]/origin", "POST [notes]/run"]);
  }
}

mod run_states {
  use super::*;
  use crate::RunState;
  use std::cell::RefCell;

  // Every RunState a space moves to, by its name.
  fn states_of(carbon: &Rc<crate::Carbon>, space: &crate::Space) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(vec![]));
    let into = Rc::clone(&seen);
    carbon.watch_run_state(&space.id, Rc::new(move |state: &RunState| into.borrow_mut().push(serde_json::to_value(state).unwrap()["state"].as_str().unwrap().to_string())));
    seen
  }

  fn insert(text: &str) -> Value {
    json!({"do": [{"insert": text}]})
  }

  #[test]
  fn a_system_failure_stops_the_space_and_every_call_to_it_until_it_is_resumed() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let states = states_of(&carbon, &space);
    let holder = crate::store::hold_lock(space.store().path());

    let error = call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", insert("a")).unwrap_err();
    assert_eq!((error.status, error.code.as_str()), (503, "SPACE_STOPPED"));
    assert_eq!(error.details.unwrap()["failure"]["code"], json!("SYSTEM_FAILURE"));
    assert!(matches!(space.run_state(), RunState::Stopped { .. }));
    for key in ["POST [notes]/run", "GET [notes]/texts"] {
      assert_eq!(call_now(&carbon, &space, &actor(&identity), key, insert("b")).unwrap_err().code, "SPACE_STOPPED", "{key}");
    }

    drop(holder);
    carbon.resume(&space);
    call_now(&carbon, &space, &actor(&identity), "POST [notes]/run", insert("c")).unwrap();
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["c"]));
    assert_eq!(logged(&space), vec!["POST [space]/origin", "POST [notes]/run"]);
    assert_eq!(*states.borrow(), vec!["stopped", "idle", "applying", "idle"]);
  }

  // A full disk inside a handler's own statement stops the space, whatever the
  // handler does with the error: the op is not refused, and applies on resume.
  #[test]
  fn an_op_a_system_failure_interrupts_stays_pending_and_applies_on_resume() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let call = crate::Call::from_key("POST [notes]/run").unwrap();
    let large = "x".repeat(64 * 1024);
    let op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"call": "POST [notes]/run", "input": insert(&large), "catch": true}]}));
    oplog::append(space.store(), &op).unwrap();
    let pages = space.store().get("PRAGMA page_count", &[]).unwrap()["page_count"].clone();
    space.store().run_batch(&format!("PRAGMA max_page_count = {pages}")).unwrap();

    carbon.drain(&space);
    assert!(matches!(space.run_state(), RunState::Stopped { error } if error.status == 503));
    assert_eq!(outcomes(&space), vec![Ok(()), Err(0)]);

    space.store().run_batch("PRAGMA max_page_count = 1073741823").unwrap();
    carbon.resume(&space);
    assert_eq!(space.run_state(), RunState::Idle);
    assert_eq!(outcomes(&space), vec![Ok(()), Ok(())]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!([large]));
  }

  // Restart with a room naming [notes] and [jots], registering only the services given.
  fn restarted_room(registered: &[&str]) -> (tempfile::TempDir, Rc<crate::Carbon>, Rc<crate::Space>, crate::Actor) {
    let (directory, carbon) = carbon();
    crate::testing::register_script(&carbon, "jots");
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let created = call_now(&carbon, &personal, &actor(&identity), "POST [@carbon]/space", json!({"name": "Room", "services": ["notes", "jots"]})).unwrap();
    let id = created.value["space"].as_str().unwrap().to_string();
    drop((personal, carbon));
    let restarted = crate::Carbon::open(&directory.path().join("carbondb.toml")).unwrap();
    for name in registered {
      crate::testing::register_script(&restarted, name);
    }
    let space = space_now(&restarted, &id).unwrap();
    (directory, restarted, space, actor(&identity))
  }

  // Submit a call, and keep its answer for whenever it comes.
  fn submitted(carbon: &Rc<crate::Carbon>, space: &Rc<crate::Space>, actor: &crate::Actor, key: &str, input: Value) -> Rc<RefCell<Option<Result<Value, u16>>>> {
    let answer = Rc::new(RefCell::new(None));
    let into = Rc::clone(&answer);
    let call = crate::Call::from_key(key).unwrap();
    carbon.submit(space, actor.clone(), call, input, Box::new(move |settled| *into.borrow_mut() = Some(settled.map(|settled| settled.value).map_err(|error| error.status))));
    answer
  }

  // While a service its space names is not set up here, the space is
  // initializing: an op is logged like any other, and it and its caller wait.
  #[test]
  fn an_op_waits_while_its_space_initializes_and_applies_once_every_service_is_set_up() {
    let (_directory, carbon, space, actor) = restarted_room(&["notes"]);
    assert_eq!(space.run_state(), RunState::Initializing { services: vec!["jots".into()] });
    let answer = submitted(&carbon, &space, &actor, "POST [notes]/run", insert("a"));
    assert!(answer.borrow().is_none());
    assert_eq!(logged(&space), vec!["POST [space]/origin", "POST [notes]/run"]);
    assert_eq!(outcomes(&space), vec![Ok(()), Err(0)]);

    crate::testing::register_script(&carbon, "jots");
    assert_eq!(space.run_state(), RunState::Idle);
    assert_eq!(answer.borrow_mut().take(), Some(Ok(json!([]))));
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["a"]));
  }

  // A call to a service this device does not run is made with no version of
  // it: it is logged like any other, then refused when its turn comes, and its
  // refusal is kept as its result.
  #[test]
  fn logs_a_call_to_a_service_this_device_does_not_run_and_refuses_it_in_turn() {
    let (_directory, carbon, space, actor) = restarted_room(&["jots"]);
    let answer = submitted(&carbon, &space, &actor, "POST [notes]/run", insert("a"));
    assert_eq!(oplog::entries(space.store()).unwrap()[1].version, crate::services::SERVICE_MISSING);

    crate::testing::register_notes(&carbon);
    assert_eq!(answer.borrow_mut().take(), Some(Err(404)));
    assert_eq!(outcomes(&space), vec![Ok(()), Err(404)]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!([]));

    let elsewhere = submitted(&carbon, &space, &actor, "POST [elsewhere]/run", insert("b"));
    assert_eq!(elsewhere.borrow_mut().take(), Some(Err(404)));
    assert_eq!(outcomes(&space), vec![Ok(()), Err(404), Err(404)]);
  }
}

mod identities {
  use super::*;
  use crate::services::carbon::Identity;

  // What a call answers in a space, as the identity.
  fn read(carbon: &Rc<crate::Carbon>, space: &Rc<crate::Space>, identity: &Identity, key: &str, input: Value) -> Value {
    call_now(carbon, space, &actor(identity), key, input).unwrap().value
  }

  #[test]
  fn sets_up_both_spaces_by_ops_and_logs_its_own_call_in_the_system_log() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(carbon.identity("alice").unwrap(), identity);
    assert_eq!(logged(carbon.system()), vec!["POST [@carbon]/identity"]);

    let identity_space = space_now(&carbon, &identity.space).unwrap();
    assert_eq!(logged(&identity_space), vec!["POST [space]/origin", "POST [identity]/origin", "POST [device]/origin", "POST [space]/ref"]);
    // The identity space is founded by its first device alone, before the identity's id exists.
    let authors: Vec<_> = oplog::entries(identity_space.store()).unwrap().into_iter().map(|op| op.author).collect();
    let id = Some(identity.id.clone());
    assert_eq!(authors, vec![None, None, id.clone(), id]);
    assert!(oplog::entries(identity_space.store()).unwrap().iter().all(|op| op.device == Some(identity.device.clone())));
    let answer = |key: &str, input: Value| read(&carbon, &identity_space, &identity, key, input);
    assert_eq!(answer("GET [space]/manage", Value::Null), json!([identity.id]));
    assert_eq!(answer("GET [identity]/origin", Value::Null), json!({"id": identity.id, "device": identity.device}));
    assert_eq!(answer("GET [device]/devices", Value::Null), json!([identity.device]));
    assert_eq!(answer("GET [space]/ref", json!({"kind": "personal", "slug": "alice"}))["space"], json!(identity.personal));

    let personal = space_now(&carbon, &identity.personal).unwrap();
    assert_eq!(logged(&personal), vec!["POST [space]/origin", "POST [device]/ref"]);
    let origin = read(&carbon, &personal, &identity, "GET [space]/origin", Value::Null);
    assert_eq!(origin, json!({"kind": "personal", "name": "alice", "services": ["device", "notes"], "manage": [identity.id]}));
    let devices = read(&carbon, &personal, &identity, "GET [device]/refs", Value::Null);
    assert_eq!(devices, json!([{"pubkey": identity.device, "name": "Pixel9", "tags": ["manage"]}]));
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
    let listed = call_now(&carbon, carbon.system(), &Actor::default(), "GET [@carbon]/identities", Value::Null).unwrap().value;
    assert!(listed[0].get("secret").is_none());
    let op = &oplog::entries(carbon.system().store()).unwrap()[0];
    let kept = serde_json::to_string(&(op, results::of(carbon.system().store(), &op.hash).unwrap())).unwrap();
    assert!(!kept.contains(&secret));
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
    assert_eq!(outcomes(carbon.system()), vec![Ok(()), Err(409), Err(400), Err(400), Err(400)]);
  }
}

mod spaces {
  use super::*;

  #[test]
  fn creates_a_space_by_a_ref_op_in_the_calling_space_and_an_origin_op_in_the_new_one() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let create = |name: &str| call_now(&carbon, &personal, &actor(&identity), "POST [@carbon]/space", json!({"name": name, "services": ["notes"]})).unwrap().value;

    let created = create("Randall's Playhouse");
    assert_eq!((created["slug"].as_str(), created["kind"].as_str()), (Some("randalls-playhouse"), Some("space")));
    assert_eq!(create("Randall's Playhouse")["slug"].as_str(), Some("randalls-playhouse-2"));

    let entries = oplog::entries(personal.store()).unwrap();
    let keys: Vec<&str> = entries.iter().map(|op| op.key.as_str()).collect();
    assert_eq!(keys[2..], ["POST [@carbon]/space", "POST [@carbon]/space"]);
    let local = &entries[2];
    assert_eq!(local.device, Some(identity.device.clone()));
    let sub_calls = oplog::sub_calls(personal.store(), &local.hash).unwrap();
    let reference = &sub_calls[0];
    assert_eq!((sub_calls.len(), reference.key.as_str()), (1, "POST [space]/ref"));
    assert_eq!((&reference.author, &reference.device), (&Some(identity.id.clone()), &Some(identity.device.clone())));
    let Some(Outcome::Applied { calls, .. }) = results::of(personal.store(), &local.hash).unwrap() else { panic!("[@carbon]/space applied") };
    assert_eq!(calls[0]["op"]["hash"], json!(reference.hash));
    assert_eq!(oplog::sub_calls(personal.store(), &entries[0].hash).unwrap(), vec![], "a replicated op carries no sub-calls");
    assert_eq!(local.hash, created["space"].as_str().unwrap());

    let room = space_now(&carbon, created["space"].as_str().unwrap()).unwrap();
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
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let stranger = Actor { identity: Some("someone-else".into()), device: None };
    let error = call_now(&carbon, &personal, &stranger, "POST [@carbon]/space", json!({"name": "Room"})).unwrap_err();
    assert_eq!(error.status, 403);
    assert_eq!(call_now(&carbon, &personal, &actor(&identity), "GET [space]/refs", Value::Null).unwrap().value, json!([]));
    assert_eq!(logged(&personal), vec!["POST [space]/origin", "POST [device]/ref", "POST [@carbon]/space"]);
    assert_eq!(outcomes(&personal), vec![Ok(()), Ok(()), Err(403)]);
  }

  #[test]
  fn refuses_calls_that_need_peers_are_not_built_or_break_the_rules() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let identity_space = space_now(&carbon, &identity.space).unwrap();
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
    let personal = space_now(&carbon, &identity.personal).unwrap();
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
