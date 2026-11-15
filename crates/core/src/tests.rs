//! Tests across the core: how calls apply, nest and roll back; what the op log
//! and results record; and what `[@carbon]` sets up, through the builtins, as ops.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::ops::results::{self, Outcome};
use crate::testing::{actor, alice, call_now, carbon, room, space_now, texts};
use crate::ops::oplog;
use crate::{Actor, Db, Routing};

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
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::testing::reopen(&directory);
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
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::testing::reopen(&directory);
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

    let restarted = crate::testing::reopen(&directory);
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
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
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
    let restarted = crate::testing::reopen(&directory);
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
    let sub_calls = oplog::sub_calls(personal.store(), &local.hash).unwrap().unwrap();
    let reference = &sub_calls[0];
    assert_eq!((sub_calls.len(), reference.key.as_str()), (1, "POST [space]/ref"));
    assert_eq!((&reference.author, &reference.device), (&Some(identity.id.clone()), &Some(identity.device.clone())));
    let Some(Outcome::Applied { calls, .. }) = results::of(personal.store(), &local.hash).unwrap() else { panic!("[@carbon]/space applied") };
    assert_eq!(calls[0]["op"]["hash"], json!(reference.hash));
    assert_eq!(oplog::sub_calls(personal.store(), &entries[0].hash).unwrap(), None, "a replicated op carries no sub-calls");
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
      (&identity_space, &alice, "POST [@carbon]/devices", json!({}), "SINGLE_NODE"),
      (&identity_space, &alice, "GET [@carbon]/identity/export", Value::Null, "NOT_BUILT"),
      (&system, &Actor::default(), "POST [@carbon]/identity", json!({"slug": "alice-2", "device": "x", "code": "scanned"}), "SINGLE_NODE"),
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
  fn says_where_a_carbon_call_made_in_the_wrong_space_is_answered() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let alice = actor(&identity);
    let system = Rc::clone(carbon.system());
    let cases = [
      (&system, Actor::default(), "POST [@carbon]/space", "In the system space, [@carbon] answers only POST [@carbon]/identity and GET [@carbon]/identities, not POST [@carbon]/space"),
      (&system, Actor::default(), "POST [@carbon]/devices", "In the system space, [@carbon] answers only POST [@carbon]/identity and GET [@carbon]/identities, not POST [@carbon]/devices"),
      (&personal, alice.clone(), "POST [@carbon]/identity", "POST [@carbon]/identity is answered in the system space, not here"),
      (&personal, alice.clone(), "GET [@carbon]/identities", "GET [@carbon]/identities is answered in the system space, not here"),
      (&personal, alice.clone(), "POST [@carbon]/devices", "POST [@carbon]/devices is answered in an identity space, not here"),
      (&personal, alice.clone(), "GET [@carbon]/identity/export", "GET [@carbon]/identity/export is answered in an identity space, not here"),
      (&personal, alice.clone(), "GET [@carbon]/nothing", "GET [@carbon]/nothing is not a call CarbonDB answers"),
    ];
    for (space, actor, key, message) in cases {
      let error = call_now(&carbon, space, &actor, key, json!({})).unwrap_err();
      assert_eq!((error.status, error.message.as_str()), (404, message), "{key}");
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

mod deadlines {
  use super::*;
  use crate::testing::{NOTES_TIMEOUT, hung, later, timed};

  #[test]
  fn an_op_not_settled_within_its_service_timeout_rolls_back_and_the_next_applies() {
    let (_directory, carbon, timers) = timed();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let nested = json!({"do": [{"insert": "a"}, {"call": "POST [notes]/run", "input": {"do": [{"insert": "b"}, {"hang": true}]}}]});
    let answer = later(|reply| carbon.submit(&personal, actor(&identity), crate::Call::from_key("POST [notes]/run").unwrap(), nested, reply));
    assert!(answer.borrow().is_none());
    assert_eq!(timers.delays(), [NOTES_TIMEOUT], "one deadline covers a call and the calls it makes");
    timers.run();
    let error = answer.borrow_mut().take().unwrap().unwrap_err();
    assert_eq!((error.status, error.code.as_str()), (504, "TIMED_OUT"));
    assert_eq!(texts(personal.store()).unwrap(), json!([]));
    assert_eq!(outcomes(&personal).last(), Some(&Err(504)));
    call_now(&carbon, &personal, &actor(&identity), "POST [notes]/run", json!({"do": [{"insert": "c"}]})).unwrap();
    assert_eq!(texts(personal.store()).unwrap(), json!(["c"]));
  }

  #[test]
  fn a_get_not_settled_in_time_answers_with_the_timeout_and_a_late_answer_is_dropped() {
    let (_directory, carbon, timers) = timed();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    hung();
    let answer = later(|reply| carbon.submit(&personal, actor(&identity), crate::Call::from_key("GET [notes]/hang").unwrap(), Value::Null, reply));
    timers.run();
    assert_eq!(answer.borrow_mut().take().unwrap().unwrap_err().status, 504);
    let [late] = hung().try_into().ok().unwrap();
    assert!(late.settle(Ok(json!("late"))).is_ok());
    assert_eq!(late.all("SELECT text FROM note", &[]).unwrap_err().status, 500, "nothing runs inside an expired call");
  }

  #[test]
  fn a_call_that_settles_cancels_its_deadline() {
    let (_directory, carbon, timers) = timed();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    assert_eq!(timers.delays(), [], "creating an identity left no deadline behind");
    call_now(&carbon, &personal, &actor(&identity), "POST [notes]/run", json!({"do": [{"insert": "a"}]})).unwrap();
    call_now(&carbon, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();
    assert_eq!(timers.delays(), []);
    assert_eq!(texts(personal.store()).unwrap(), json!(["a"]));
  }
}

// Replicas: the same ops delivered in different orders converge on the same
// state, as though each had applied them in the shared order.
mod replicas {
  use super::*;
  use crate::ops::op::stamp;
  use crate::ops::oplog::LogRow;
  use crate::testing::{now, register_script};
  use crate::{Call, Carbon, Space};

  const ROOM: &str = "room";

  // An op in a scenario: when it was made, from which of alice's devices, and its call.
  struct Made {
    time: u64,
    device: &'static str,
    key: &'static str,
    input: Value,
  }

  fn made(time: u64, device: &'static str, key: &'static str, input: Value) -> Made {
    Made { time, device, key, input }
  }

  // A `[notes]` or `[todo]` script made at `time`.
  fn script(time: u64, service: &'static str, steps: Value) -> Made {
    let key = if service == "todo" { "POST [todo]/run" } else { "POST [notes]/run" };
    made(time, if time.is_multiple_of(2) { "phone" } else { "laptop" }, key, json!({"do": steps}))
  }

  // The row an op travels as, stamped with the version of its service this replica runs.
  fn row(carbon: &Carbon, made: &Made) -> LogRow {
    let call = Call::from_key(made.key).unwrap();
    let actor = Actor { identity: Some("alice".into()), device: Some(made.device.into()) };
    LogRow::new(stamp(&actor, &call, &made.input, &carbon.services().version(&call.service), made.time))
  }

  // The room's origin, which every replica receives first, as p2panda's
  // causal delivery guarantees: every op in a space follows its origin.
  fn origin() -> Made {
    made(1, "laptop", "POST [space]/origin", json!({"kind": "space", "name": "Room", "services": ["notes", "todo"], "manage": ["alice"]}))
  }

  // A fresh replica running `[notes]` and `[todo]`.
  fn device() -> (tempfile::TempDir, Rc<Carbon>) {
    let (directory, carbon) = carbon();
    register_script(&carbon, "todo");
    (directory, carbon)
  }

  fn receive(carbon: &Rc<Carbon>, row: LogRow) {
    now(|received| carbon.receive(ROOM, row, received)).unwrap();
  }

  // A replica that received the room's origin, then these rows in this order.
  fn replica(rows: &[&LogRow]) -> (tempfile::TempDir, Rc<Carbon>, Rc<Space>) {
    let (directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    rows.iter().for_each(|row| receive(&carbon, (*row).clone()));
    let space = space_now(&carbon, ROOM).unwrap();
    (directory, carbon, space)
  }

  // A service's notes, as they stand in a replica.
  fn notes(space: &Space, service: &str) -> Value {
    let rows = space.store().reader().unwrap().all(&format!("SELECT text FROM \"{service}/db/note\" ORDER BY text"), &[]).unwrap();
    Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect())
  }

  // What a replica holds: each service's notes, and what became of each op in the shared order.
  fn state(space: &Space) -> Value {
    let ops = oplog::entries(space.store()).unwrap();
    let results: Vec<Value> = ops.iter().map(|op| json!([op.key, results::of(space.store(), &op.hash).unwrap()])).collect();
    json!({"notes": notes(space, "notes"), "todo": notes(space, "todo"), "results": results})
  }

  // Every order of the indices 0..n.
  fn orders(n: usize) -> Vec<Vec<usize>> {
    match n {
      0 => vec![vec![]],
      n => orders(n - 1).into_iter().flat_map(|order| (0..n).map(move |at| [&order[..at], &[n - 1], &order[at..]].concat())).collect(),
    }
  }

  // Deliver a scenario's ops to a replica in every order, and check each ends
  // where the replica that received them in the shared order ends, holding
  // the notes expected.
  fn converges(name: &str, ops: &[Made], expected: Value) {
    let (_directory, author) = device();
    let rows: Vec<LogRow> = ops.iter().map(|made| row(&author, made)).collect();
    let (_in_order, _carbon, in_order) = replica(&rows.iter().collect::<Vec<_>>());
    let settled = state(&in_order);
    assert_eq!(json!({"notes": settled["notes"], "todo": settled["todo"]}), expected, "{name}: in order");
    for order in orders(rows.len()) {
      let delivered: Vec<&LogRow> = order.iter().map(|at| &rows[*at]).collect();
      let (_directory, _carbon, space) = replica(&delivered);
      assert_eq!(state(&space), settled, "{name}: delivered in order {order:?}");
    }
  }

  #[test]
  fn ops_delivered_in_any_order_converge_on_the_state_of_the_shared_order() {
    let scenarios = [
      (
        "an edit made after a concurrent delete is refused everywhere",
        vec![script(10, "notes", json!([{"insert": "a"}])), script(20, "notes", json!([{"delete": "a"}])), script(30, "notes", json!([{"replace": ["a", "b"]}]))],
        json!({"notes": [], "todo": []}),
      ),
      (
        "an edit refused before the note arrived applies once it has",
        vec![script(10, "notes", json!([{"insert": "a"}])), script(20, "notes", json!([{"replace": ["a", "b"]}]))],
        json!({"notes": ["b"], "todo": []}),
      ),
      (
        "a read sees every op before it, whenever they arrived",
        vec![script(10, "notes", json!([{"insert": "a"}])), script(20, "notes", json!([{"insert": "b"}, {"read": true}])), script(30, "notes", json!([{"read": true}]))],
        json!({"notes": ["a", "b"], "todo": []}),
      ),
      (
        "ops on different services leave each other be",
        vec![script(10, "notes", json!([{"insert": "a"}])), script(20, "todo", json!([{"insert": "t"}])), script(30, "notes", json!([{"read": true}]))],
        json!({"notes": ["a"], "todo": ["t"]}),
      ),
      (
        "a call into another service rewinds with both",
        vec![
          script(10, "todo", json!([{"insert": "t"}])),
          script(20, "notes", json!([{"insert": "n"}, {"call": "POST [todo]/run", "input": {"do": [{"read": true}, {"insert": "from-notes"}]}}])),
          script(30, "todo", json!([{"replace": ["t", "t2"]}])),
        ],
        json!({"notes": ["n"], "todo": ["from-notes", "t2"]}),
      ),
      (
        "a branch that reaches another service only at its true place",
        vec![
          script(10, "notes", json!([{"unless": "x", "do": [{"call": "POST [todo]/run", "input": {"do": [{"insert": "early"}]}}]}])),
          script(20, "notes", json!([{"insert": "x"}])),
          script(30, "todo", json!([{"insert": "t"}, {"read": true}])),
        ],
        json!({"notes": ["x"], "todo": ["early", "t"]}),
      ),
      (
        "a value kept unique by a query, never by a constraint, is inserted only by the first op",
        vec![script(10, "notes", json!([{"unless": "a", "do": [{"insert": "a"}]}])), script(20, "notes", json!([{"unless": "a", "do": [{"insert": "a"}]}]))],
        json!({"notes": ["a"], "todo": []}),
      ),
      (
        "an insert before an edit of another row, which the edit looked up by its key",
        vec![
          script(5, "notes", json!([{"set": ["k", "first"]}])),
          script(10, "notes", json!([{"insert": "a"}])),
          script(20, "notes", json!([{"set": ["k", "second"]}])),
        ],
        json!({"notes": ["a", "second"], "todo": []}),
      ),
      (
        "an edit of a row by its key before the insert of that row",
        vec![script(10, "notes", json!([{"set": ["k", "edited"]}])), script(20, "notes", json!([{"set": ["k", "inserted"]}]))],
        json!({"notes": ["inserted"], "todo": []}),
      ),
      (
        "inserts of different rows into one table",
        vec![script(10, "notes", json!([{"insert": "a"}])), script(20, "notes", json!([{"insert": "b"}])), script(30, "notes", json!([{"insert": "c"}]))],
        json!({"notes": ["a", "b", "c"], "todo": []}),
      ),
    ];
    for (name, ops, expected) in scenarios {
      converges(name, &ops, expected);
    }
  }

  // The refs a replica's room holds, by the space each points to, with its slug.
  fn refs(carbon: &Rc<Carbon>, space: &Rc<Space>) -> Value {
    let refs = call_now(carbon, space, &Actor::default(), "GET [space]/refs", Value::Null).unwrap().value;
    Value::Object(refs.as_array().unwrap().iter().map(|reference| (reference["space"].as_str().unwrap().to_string(), reference["slug"].clone())).collect())
  }

  #[test]
  fn a_local_op_replays_its_sub_calls_on_every_replica_and_never_runs_itself() {
    let (_directory, author) = device();
    let reference = |time: u64, device: &'static str, space: &str| made(time, device, "POST [space]/ref", json!({"space": space, "kind": "space", "name": "Lobby", "slug": null, "cascade": []}));
    let direct = row(&author, &reference(10, "phone", "elsewhere"));
    let local = LogRow { sub_calls: Some(vec![row(&author, &reference(20, "laptop", "child")).op]), ..row(&author, &made(20, "laptop", "POST [@carbon]/space", json!({"name": "Lobby"}))) };
    for order in [[&direct, &local], [&local, &direct]] {
      let (directory, carbon, space) = replica(&order);
      assert_eq!(refs(&carbon, &space), json!({"elsewhere": "lobby", "child": "lobby-2"}), "delivered local op first: {}", order[0] == &local);
      assert!(!directory.path().join("data/child.db").exists() && !directory.path().join(format!("data/{}.db", local.op.hash)).exists(), "the local op itself never ran");
    }
  }

  #[test]
  fn a_local_op_rewound_on_the_device_that_made_it_replays_its_sub_calls_and_never_runs_again() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let child = call_now(&carbon, &space, &actor(&identity), "POST [@carbon]/space", json!({"name": "Lobby"})).unwrap().value;
    let local = oplog::entries(space.store()).unwrap().into_iter().find(|op| op.key == "POST [@carbon]/space").unwrap();
    // Made on another of alice's devices at the same moment, it sorts just before the local op.
    let late = Actor { identity: Some(identity.id.clone()), device: Some("0".into()) };
    let input = json!({"space": "elsewhere", "kind": "space", "name": "Lobby", "slug": null, "cascade": []});
    let reference = stamp(&late, &Call::from_key("POST [space]/ref").unwrap(), &input, &carbon.services().version("space"), local.time);
    now(|received| carbon.receive(&space.id, LogRow::new(reference), received)).unwrap();

    assert_eq!(refs(&carbon, &space), json!({"elsewhere": "lobby", child["space"].as_str().unwrap(): "lobby-2"}));
    let databases = std::fs::read_dir(directory.path().join("data")).unwrap().filter(|entry| entry.as_ref().unwrap().path().extension().is_some_and(|extension| extension == "db")).count();
    assert_eq!(databases, 5, "the system database, alice's two spaces, the room and the child, and no second child");
    assert_eq!(outcomes(&space), vec![Ok(()), Ok(()), Ok(())]);
  }

  #[test]
  fn a_watch_shows_the_state_a_rewind_settles_on() {
    let (_directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(20, "notes", json!([{"replace": ["a", "b"]}]))));
    receive(&carbon, row(&carbon, &script(10, "notes", json!([{"insert": "a"}]))));
    assert_eq!(seen.borrow().last(), Some(&json!(["b"])));
  }

  #[test]
  fn a_late_insert_of_another_row_applies_in_place_and_nothing_re_runs() {
    let (_directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(20, "notes", json!([{"insert": "b"}]))));
    receive(&carbon, row(&carbon, &script(10, "notes", json!([{"insert": "a"}]))));
    // A rewind would have shown the later insert undone, then applied again.
    assert_eq!(*seen.borrow(), vec![json!([]), json!(["b"]), json!(["a", "b"])]);
  }

  #[test]
  fn a_late_insert_applies_in_place_before_an_edit_that_looked_up_another_row_by_its_key() {
    let (_directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(5, "notes", json!([{"set": ["k", "first"]}]))));
    receive(&carbon, row(&carbon, &script(20, "notes", json!([{"set": ["k", "second"]}]))));
    receive(&carbon, row(&carbon, &script(10, "notes", json!([{"insert": "a"}]))));
    // A rewind would have shown the edit undone, then made again.
    assert_eq!(*seen.borrow(), vec![json!([]), json!(["first"]), json!(["second"]), json!(["a", "second"])]);
  }

  #[test]
  fn a_call_made_after_receiving_an_op_sorts_after_it() {
    let (_directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    // A minute ahead of this device's clock, within how far clocks drift.
    let future = crate::ops::clock::wall() + 60_000;
    receive(&carbon, row(&carbon, &script(future, "notes", json!([{"insert": "a"}]))));
    let space = space_now(&carbon, ROOM).unwrap();
    call_now(&carbon, &space, &Actor { identity: Some("alice".into()), device: Some("pixel".into()) }, "POST [notes]/run", json!({"do": [{"read": true}]})).unwrap();
    let ops = oplog::entries(space.store()).unwrap();
    assert!(ops[2].time > future);
    assert_eq!(results::of(space.store(), &ops[2].hash).unwrap(), Some(Outcome::Applied { output: json!([["a"]]), calls: json!([]) }));
  }

  #[test]
  fn a_row_whose_time_is_too_far_ahead_or_impossible_is_refused_unlogged_and_leaves_the_clock_alone() {
    let (_directory, carbon) = device();
    receive(&carbon, row(&carbon, &origin()));
    let wall = crate::ops::clock::wall();
    let cases = [("too far ahead, for now", wall + 3_600_000, 425), ("never", crate::ops::clock::NEVER, 400)];
    for (name, time, status) in cases {
      let refused = now(|received| carbon.receive(ROOM, row(&carbon, &script(time, "notes", json!([{"insert": "a"}]))), received));
      assert_eq!(refused.unwrap_err().status, status, "{name}");
    }
    let space = space_now(&carbon, ROOM).unwrap();
    assert_eq!(oplog::entries(space.store()).unwrap().len(), 1, "only the origin was logged");
    let next = carbon.stamp(&Actor::default(), &Call::from_key("POST [notes]/run").unwrap(), &json!({})).time;
    assert!(next < wall + 60_000, "the clock never took in a refused time");
  }
}
