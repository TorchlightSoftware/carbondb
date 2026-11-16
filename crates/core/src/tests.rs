//! Tests across the core: how calls apply, nest and roll back; what the op log
//! and results record; what `[@carbon]` founds, through `[channel]` and
//! `[identity]`, as ops; how channels name and create services; how effects
//! are made once and replayed; and how replicas converge.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::ops::results::{self, Outcome};
use crate::testing::{actor, alice, call_now, carbon, on, room, room_channel, service, space_now, texts};
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
    assert_eq!(wrote.tables, vec!["note"]);
    let read = call_now(&carbon, &space, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();
    assert_eq!(read.tables, vec!["note"]);
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

    assert_eq!(logged(&space), vec!["POST [notes]/run", "POST [notes]/run"]);
    assert_eq!(outcomes(&space), vec![Ok(()), Err(409)]);
    let applied = &oplog::entries(space.store()).unwrap()[0];
    assert_eq!((&applied.author, &applied.device, &applied.input, applied.version.as_str()), (&Some(identity.id.clone()), &Some(identity.device.clone()), &script, "notes-v1"));
    let calls = json!([{"key": "POST [notes]/run", "input": {"do": [{"insert": "a"}]}, "output": []}]);
    assert_eq!(results::of(space.store(), &applied.hash).unwrap(), Some(Outcome::Applied { output: json!([[]]), calls }));
    assert_eq!(oplog::effects(space.store(), &applied.hash).unwrap(), Some(vec![]), "a call to the same service is no effect");
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["a"]));
  }

  #[test]
  fn applies_an_op_left_without_a_result_once_its_service_is_registered_again() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let call = crate::Call::from_key("POST [notes]/run").unwrap();
    let op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"insert": "after a crash"}]}), "notes");
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::testing::reopen(&directory);
    let space = space_now(&restarted, &id).unwrap();
    assert_eq!(space.run_state(), crate::RunState::Initializing { services: vec!["notes".into()] });
    assert_eq!(outcomes(&space), vec![Err(0)]);
    crate::testing::register_notes(&restarted);
    assert_eq!(outcomes(&space), vec![Ok(())]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!(["after a crash"]));
  }

  #[test]
  fn refuses_an_op_made_for_another_version_of_its_service() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let call = crate::Call::from_key("POST [notes]/run").unwrap();
    let op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"insert": "old"}]}), "notes");
    let op = crate::Op::new(op.key, op.input, op.author, op.device, "notes-v0".into(), op.time, op.hash);
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
    let id = space.id.clone();
    drop((space, carbon));

    let restarted = crate::testing::reopen(&directory);
    crate::testing::register_notes(&restarted);
    let space = space_now(&restarted, &id).unwrap();
    assert_eq!(outcomes(&space), vec![Err(409)]);
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
    assert_eq!(logged(&space), vec!["POST [notes]/run"]);
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
    assert_eq!(logged(&space), vec!["POST [notes]/run"]);
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
    let op = carbon.stamp(&actor(&identity), &call, &json!({"do": [{"call": "POST [notes]/run", "input": insert(&large), "catch": true}]}), "notes");
    oplog::append(space.store(), &oplog::LogRow::new(op.clone())).unwrap();
    let pages = space.store().get("PRAGMA page_count", &[]).unwrap()["page_count"].clone();
    space.store().run_batch(&format!("PRAGMA max_page_count = {pages}")).unwrap();

    carbon.drain(&space);
    assert!(matches!(space.run_state(), RunState::Stopped { error } if error.status == 503));
    assert_eq!(outcomes(&space), vec![Err(0)]);

    space.store().run_batch("PRAGMA max_page_count = 1073741823").unwrap();
    carbon.resume(&space);
    assert_eq!(space.run_state(), RunState::Idle);
    assert_eq!(outcomes(&space), vec![Ok(())]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!([large]));
  }

  // Submit a call, and keep its answer for whenever it comes.
  fn submitted(carbon: &Rc<crate::Carbon>, space: &Rc<crate::Space>, actor: &crate::Actor, key: &str, input: Value) -> Rc<RefCell<Option<Result<Value, u16>>>> {
    let answer = Rc::new(RefCell::new(None));
    let into = Rc::clone(&answer);
    let call = crate::Call::from_key(key).unwrap();
    carbon.submit(space, actor.clone(), call, input, Box::new(move |settled| *into.borrow_mut() = Some(settled.map(|settled| settled.value).map_err(|error| error.status))));
    answer
  }

  // A call to a service this device does not run is made with no version of
  // it: it is logged like any other while its space waits for the service,
  // then refused when its turn comes, and its refusal is kept as its result.
  #[test]
  fn logs_a_call_to_a_service_this_device_does_not_run_and_refuses_it_in_turn() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let space = room(&carbon, &identity);
    let id = space.id.clone();
    drop((space, carbon));
    let restarted = crate::testing::reopen(&directory);
    let space = space_now(&restarted, &id).unwrap();
    let answer = submitted(&restarted, &space, &actor(&identity), "POST [notes]/run", insert("a"));
    assert!(answer.borrow().is_none(), "its space waits for its service");
    assert_eq!(oplog::entries(space.store()).unwrap()[0].version, crate::services::SERVICE_MISSING);

    crate::testing::register_notes(&restarted);
    assert_eq!(answer.borrow_mut().take(), Some(Err(404)));
    assert_eq!(outcomes(&space), vec![Err(404)]);
    assert_eq!(texts(&space.store().reader().unwrap()).unwrap(), json!([]));
    let elsewhere = submitted(&restarted, &space, &actor(&identity), "POST [elsewhere]/run", insert("b"));
    assert_eq!(elsewhere.borrow_mut().take(), Some(Err(404)), "a space answers only its own service");
  }
}

mod identities {
  use super::*;

  #[test]
  fn founds_its_own_channel_sealed_with_identity_and_a_personal_channel_by_ops() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(carbon.identity("alice").unwrap(), identity);
    assert_eq!(logged(carbon.system()), vec!["POST [@carbon]/identity"]);
    let founding = &oplog::entries(carbon.system().store()).unwrap()[0];
    let effects: Vec<String> = oplog::effects(carbon.system().store(), &founding.hash).unwrap().unwrap().into_iter().map(|made| format!("{} {}", made.name, made.input["key"].as_str().unwrap_or(""))).collect();
    let expected = [
      "create ",
      "call POST [channel]/origin",
      "call POST [channel]/services",
      "call POST [identity]/origin",
      "call POST [channel]/channel",
      "call POST [channel]/services",
      "call POST [channel]/seal",
    ];
    assert_eq!(effects, expected, "its own op records every effect it made");

    let own = space_now(&carbon, &identity.channel).unwrap();
    assert_eq!(logged(&own), vec!["POST [channel]/origin", "POST [channel]/services", "POST [channel]/channel", "POST [channel]/seal"]);
    assert!(oplog::entries(own.store()).unwrap().iter().all(|op| op.author == Some(identity.id.clone()) && op.device == Some(identity.device.clone())));
    let answer = |channel: &str, key: &str, input: Value| on(&carbon, channel, &identity, key, input).unwrap();
    assert_eq!(answer(&identity.channel, "GET [channel]/origin", Value::Null), json!({"kind": "identity", "name": "alice", "manage": [identity.id]}));
    assert_eq!(answer(&identity.channel, "GET [identity]/origin", Value::Null), json!({"id": identity.id, "device": identity.device}));
    assert_eq!(answer(&identity.channel, "GET [identity]/devices", Value::Null), json!([{"pubkey": identity.device, "name": "Pixel9", "tags": ["manage"]}]));
    assert_eq!(answer(&identity.channel, "GET [channel]/ref", json!({"kind": "personal", "slug": "alice"}))["channel"], json!(identity.personal));
    let sealed = on(&carbon, &identity.channel, &identity, "POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": "Sales"})).unwrap_err();
    assert_eq!((sealed.status, sealed.message.as_str()), (403, "This channel is sealed and takes no further changes"));

    assert_eq!(answer(&identity.personal, "GET [channel]/origin", Value::Null), json!({"kind": "personal", "name": "alice", "manage": [identity.id]}));
    let services = answer(&identity.personal, "GET [channel]/services", Value::Null);
    assert_eq!((services[0]["name"].clone(), services[0]["version"].clone()), (json!("notes"), json!("notes-v1")));
    assert_eq!(answer(&identity.personal, "GET [notes]/texts", Value::Null), json!([]));
  }

  #[test]
  fn renames_a_device_as_the_identity_itself_and_adding_one_needs_peers() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let renamed = on(&carbon, &identity.channel, &identity, &format!("PUT [identity]/devices/{}", identity.device), json!({"name": "Phone", "tags": []})).unwrap();
    assert_eq!(renamed, json!({"pubkey": identity.device, "name": "Phone", "tags": []}));
    let stranger = Actor { identity: Some("sally".into()), device: None };
    let devices = service(&carbon, &identity.channel, "identity");
    let refused = call_now(&carbon, &devices, &stranger, &format!("PUT [identity]/devices/{}", identity.device), json!({"name": "Mine"})).unwrap_err();
    assert_eq!(refused.status, 403);
    let enrolling = on(&carbon, &identity.channel, &identity, "POST [identity]/devices", json!({"name": "Laptop"})).unwrap_err();
    assert_eq!((enrolling.status, enrolling.code.as_str()), (503, "UNREACHABLE"));
  }

  #[test]
  fn only_the_system_database_keeps_the_secret() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let row = carbon.system().store().reader().unwrap().get("SELECT secret FROM carbon_system_identity", &[]).unwrap();
    let secret = row["secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);
    for entry in std::fs::read_dir(directory.path().join("data")).unwrap() {
      let path = entry.unwrap().path();
      if path.extension().is_some_and(|extension| extension == "db") && path.file_name().unwrap() != "system.db" {
        assert!(!String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains(&secret), "{}", path.display());
      }
    }
    let listed = call_now(&carbon, carbon.system(), &Actor::default(), "GET [@carbon]/identities", Value::Null).unwrap().value;
    assert_eq!(listed[0]["personal"], json!(identity.personal));
    assert!(listed[0].get("secret").is_none());
    let op = &oplog::entries(carbon.system().store()).unwrap()[0];
    let kept = serde_json::to_string(&(op, results::of(carbon.system().store(), &op.hash).unwrap(), oplog::effects(carbon.system().store(), &op.hash).unwrap())).unwrap();
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
      (json!({"slug": "alice-2", "device": "x", "code": "scanned"}), 501),
    ];
    for (input, status) in cases {
      let error = call_now(&carbon, carbon.system(), &Actor::default(), "POST [@carbon]/identity", input.clone()).unwrap_err();
      assert_eq!(error.status, status, "{input}");
    }
    assert_eq!(outcomes(carbon.system()), vec![Ok(()), Err(409), Err(400), Err(400), Err(400), Err(501)]);
  }
}

mod channels {
  use super::*;

  #[test]
  fn creates_a_channel_inside_another_and_points_to_it() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let create = |name: &str| on(&carbon, &identity.personal, &identity, "POST [channel]/channel", json!({"name": name})).unwrap();
    let created = create("Randall's Playhouse");
    assert_eq!((created["slug"].as_str(), created["kind"].as_str()), (Some("randalls-playhouse"), Some("channel")));
    assert_eq!(create("Randall's Playhouse")["slug"].as_str(), Some("randalls-playhouse-2"));
    let channel = created["channel"].as_str().unwrap();
    assert_eq!(carbon.find_ref(&identity.personal, "channel", "randalls-playhouse").unwrap()["channel"], json!(channel));

    let playhouse = space_now(&carbon, channel).unwrap();
    assert_eq!(logged(&playhouse), vec!["POST [channel]/origin"]);
    assert_eq!(playhouse.runs.channel.as_deref(), Some(channel), "a channel's space belongs to itself");
    let status = call_now(&carbon, carbon.system(), &actor(&identity), "GET [@carbon]/status", json!({"channel": channel})).unwrap().value;
    assert_eq!(
      (&status["name"], &status["kind"], &status["access"], &status["sync"], &status["peers"], &status["services"]),
      (&json!("Randall's Playhouse"), &json!("channel"), &json!(["manage"]), &json!("off"), &json!([]), &json!([]))
    );
  }

  #[test]
  fn names_a_service_it_creates_and_resolves_calls_to_it() {
    let (_directory, carbon) = carbon();
    crate::testing::register_script(&carbon, "jots");
    let identity = alice(&carbon);
    let room = room_channel(&carbon, &identity);
    let named = on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "scratch", "package": "jots"})).unwrap();
    assert_eq!(named["version"], json!("jots-v1"));
    on(&carbon, &room, &identity, "POST [scratch]/run", json!({"do": [{"insert": "s"}]})).unwrap();
    assert_eq!(on(&carbon, &room, &identity, "GET [scratch]/texts", Value::Null).unwrap(), json!(["s"]));
    assert_eq!(texts(&service(&carbon, &room, "notes").store().reader().unwrap()).unwrap(), json!([]), "each service keeps its own tables");
    let services: Vec<Value> = on(&carbon, &room, &identity, "GET [channel]/services", Value::Null).unwrap().as_array().unwrap().iter().map(|named| named["name"].clone()).collect();
    assert_eq!(services, [json!("notes"), json!("scratch")]);
    let refused = [
      (json!({"name": "notes"}), 409),
      (json!({"name": "channel"}), 409),
      (json!({"name": "crm"}), 404),
    ];
    for (input, status) in refused {
      assert_eq!(on(&carbon, &room, &identity, "POST [channel]/services", input.clone()).unwrap_err().status, status, "{input}");
    }
    assert_eq!(on(&carbon, &room, &identity, "GET [elsewhere]/texts", Value::Null).unwrap_err().status, 404);
  }

  #[test]
  fn only_a_manager_changes_a_channel() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let stranger = Actor { identity: Some("someone-else".into()), device: None };
    let error = call_now(&carbon, &personal, &stranger, "POST [channel]/channel", json!({"name": "Room"})).unwrap_err();
    assert_eq!(error.status, 403);
    let leaving = call_now(&carbon, &personal, &stranger, "POST [channel]/leave", json!({})).unwrap_err();
    assert_eq!(leaving.code, "UNREACHABLE", "leaving needs no manage, only peers");
    assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/refs", Value::Null).unwrap(), json!([]));
  }

  #[test]
  fn refuses_what_needs_peers_and_what_breaks_the_rules() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let alice_on = |key: &str, input: Value| on(&carbon, &identity.personal, &identity, key, input).unwrap_err().code;
    let cases = [
      ("POST [channel]/members", json!({"contact": "sally", "access": "write"}), "UNREACHABLE"),
      ("POST [channel]/join", json!({"code": "scanned"}), "UNREACHABLE"),
      ("POST [channel]/leave", json!({}), "UNREACHABLE"),
      ("POST [channel]/manage", json!({"add": ["sally"]}), "UNREACHABLE"),
      ("DELETE [channel]/members/sally", Value::Null, "UNREACHABLE"),
      ("DELETE [channel]/invitations/i1", Value::Null, "UNREACHABLE"),
      ("POST [channel]/origin", json!({"kind": "team", "name": "x", "manage": []}), "CONFLICT"),
      ("POST [channel]/ref", json!({"channel": "c1"}), "BAD_REQUEST"),
      ("POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": "!!"}), "BAD_REQUEST"),
      ("GET [channel]/nothing", Value::Null, "NOT_FOUND"),
      ("GET [channel]/ref", json!({"kind": "team", "slug": "sales"}), "NOT_FOUND"),
      ("GET [@carbon]/status", json!({}), "NOT_FOUND"),
    ];
    for (key, input, code) in cases {
      assert_eq!(alice_on(key, input), code, "{key}");
    }
    let system = |key: &str, input: Value| call_now(&carbon, carbon.system(), &actor(&identity), key, input).unwrap_err().code;
    assert_eq!(system("POST [@carbon]/sync/start", json!({"channel": identity.personal})), "UNREACHABLE");
    assert_eq!(system("GET [@carbon]/status", json!({})), "BAD_REQUEST");
    assert_eq!(system("GET [@carbon]/nothing", Value::Null), "NOT_FOUND");
    assert_eq!(system("GET [channel]/refs", Value::Null), "NOT_FOUND");
    assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/members", Value::Null).unwrap(), json!([{"identity": identity.id, "access": "manage"}]));
    assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/invitations", Value::Null).unwrap(), json!([]));
  }

  #[test]
  fn a_ref_to_the_same_channel_replaces_the_last() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    for name in ["Sales", "Sales West"] {
      on(&carbon, &identity.personal, &identity, "POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": name})).unwrap();
    }
    let refs = on(&carbon, &identity.personal, &identity, "GET [channel]/refs", Value::Null).unwrap();
    assert_eq!(refs, json!([{"channel": "c1", "kind": "team", "name": "Sales West", "slug": "sales-west", "cascade": []}]));
  }

  #[test]
  fn deletes_this_devices_copy_of_a_channel_and_its_services() {
    let (directory, carbon) = carbon();
    let identity = alice(&carbon);
    let room = room_channel(&carbon, &identity);
    let notes = service(&carbon, &room, "notes").id.clone();
    call_now(&carbon, carbon.system(), &actor(&identity), "DELETE [@carbon]/data", json!({"channel": room})).unwrap();
    for id in [&room, &notes] {
      assert!(!directory.path().join("data").join(format!("{id}.db")).exists(), "{id}");
      assert_eq!(space_now(&carbon, id).err().map(|error| error.status), Some(404), "{id}");
    }
  }
}

mod effects {
  use super::*;
  use crate::ops::effects::Made;
  use crate::testing::{now, register_script, register_script_making};

  // A room running `[notes]` and `[jots]`, and alice.
  fn room_with_jots(always: &[&str]) -> (tempfile::TempDir, Rc<crate::Carbon>, crate::Identity, String) {
    let (directory, carbon) = carbon();
    register_script(&carbon, "jots");
    register_script_making(&carbon, "loud", always);
    let identity = alice(&carbon);
    let room = room_channel(&carbon, &identity);
    on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "jots"})).unwrap();
    on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "loud"})).unwrap();
    (directory, carbon, identity, room)
  }

  // A row arriving from alice's other device, late: before everything here.
  fn late(carbon: &Rc<crate::Carbon>, space: &crate::Space, script: Value, effects: Option<Vec<Made>>) {
    let call = crate::Call::from_key(&format!("POST [{}]/run", space.runs.name)).unwrap();
    let actor = Actor { identity: Some("alice".into()), device: Some("laptop".into()) };
    let op = crate::ops::op::stamp(&actor, &call, &json!({"do": script}), &carbon.services().version(&space.runs.package), 1);
    now(|received| carbon.receive(&space.id, oplog::LogRow { op, effects }, received)).unwrap();
  }

  #[test]
  fn a_call_to_another_service_is_made_once_and_replayed_after_a_rewind() {
    let (_directory, carbon, identity, room) = room_with_jots(&[]);
    let script = json!({"do": [{"read": true}, {"call": "POST [jots]/run", "input": {"do": [{"insert": "j"}]}}]});
    on(&carbon, &room, &identity, "POST [notes]/run", script).unwrap();
    let (notes, jots) = (service(&carbon, &room, "notes"), service(&carbon, &room, "jots"));
    let made = &oplog::entries(notes.store()).unwrap()[0];
    let recorded = oplog::effects(notes.store(), &made.hash).unwrap().unwrap();
    assert_eq!((recorded.len(), recorded[0].name.as_str(), recorded[0].output.clone()), (1, "call", Ok(json!([]))));
    assert_eq!(logged(&jots), vec!["POST [jots]/run"], "the call is a message, an op in the other service");

    late(&carbon, &notes, json!([{"insert": "early"}]), Some(vec![]));
    assert_eq!(texts(&notes.store().reader().unwrap()).unwrap(), json!(["early"]));
    assert_eq!(outcomes(&notes), vec![Ok(()), Ok(())], "the rewound op ran again at its place");
    assert_eq!(logged(&jots), vec!["POST [jots]/run"], "its replay got the recorded answer and posted nothing");
    assert_eq!(texts(&jots.store().reader().unwrap()).unwrap(), json!(["j"]));
  }

  #[test]
  fn an_effect_declared_always_is_made_again_on_every_run() {
    let (_directory, carbon, identity, room) = room_with_jots(&["call"]);
    let script = json!({"do": [{"read": true}, {"call": "POST [jots]/run", "input": {"do": [{"insert": "j"}]}}]});
    on(&carbon, &room, &identity, "POST [loud]/run", script).unwrap();
    let (loud, jots) = (service(&carbon, &room, "loud"), service(&carbon, &room, "jots"));
    assert_eq!(oplog::effects(loud.store(), &oplog::entries(loud.store()).unwrap()[0].hash).unwrap(), Some(vec![]), "nothing is recorded");
    late(&carbon, &loud, json!([{"insert": "early"}]), Some(vec![]));
    assert_eq!(logged(&jots), vec!["POST [jots]/run", "POST [jots]/run"]);
  }

  #[test]
  fn a_received_op_replays_the_effects_its_row_carries_and_one_beyond_them_is_unreachable() {
    let (_directory, carbon, _identity, room) = room_with_jots(&[]);
    let (notes, jots) = (service(&carbon, &room, "notes"), service(&carbon, &room, "jots"));
    let answered = Made { name: "call".into(), input: json!({"key": "GET [jots]/texts"}), output: Ok(json!(["from the author"])) };
    late(&carbon, &notes, json!([{"call": "GET [jots]/texts"}, {"call": "GET [jots]/texts", "catch": true}]), Some(vec![answered]));
    let op = &oplog::entries(notes.store()).unwrap()[0];
    let Some(Outcome::Applied { output, .. }) = results::of(notes.store(), &op.hash).unwrap() else { panic!("applied") };
    assert_eq!(output, json!([["from the author"], {"error": 503}]));
    assert_eq!(logged(&jots), Vec::<String>::new());
  }
}

mod deadlines {
  use super::*;
  use crate::testing::{NOTES_TIMEOUT, hung, later, personal_notes, timed};

  #[test]
  fn an_op_not_settled_within_its_service_timeout_rolls_back_and_the_next_applies() {
    let (_directory, carbon, timers) = timed();
    let identity = alice(&carbon);
    let personal = personal_notes(&carbon, &identity);
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
    let personal = personal_notes(&carbon, &identity);
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
    let personal = personal_notes(&carbon, &identity);
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
  use crate::space::Runs;
  use crate::testing::now;
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

  // A `[notes]` script made at `time`.
  fn script(time: u64, steps: Value) -> Made {
    made(time, if time.is_multiple_of(2) { "phone" } else { "laptop" }, "POST [notes]/run", json!({"do": steps}))
  }

  // The row an op travels as, stamped with the version of its service this replica runs.
  fn row(carbon: &Carbon, made: &Made) -> LogRow {
    let call = Call::from_key(made.key).unwrap();
    let actor = Actor { identity: Some("alice".into()), device: Some(made.device.into()) };
    LogRow::new(stamp(&actor, &call, &made.input, &carbon.services().version("notes"), made.time))
  }

  // A fresh replica holding the room, a `[notes]` service.
  fn device() -> (tempfile::TempDir, Rc<Carbon>) {
    let (directory, carbon) = carbon();
    carbon.create_space(ROOM, Runs { package: "notes".into(), name: "notes".into(), channel: None }).unwrap();
    (directory, carbon)
  }

  fn receive(carbon: &Rc<Carbon>, row: LogRow) {
    now(|received| carbon.receive(ROOM, row, received)).unwrap();
  }

  // A replica that received these rows in this order.
  fn replica(rows: &[&LogRow]) -> (tempfile::TempDir, Rc<Carbon>, Rc<Space>) {
    let (directory, carbon) = device();
    rows.iter().for_each(|row| receive(&carbon, (*row).clone()));
    let space = space_now(&carbon, ROOM).unwrap();
    (directory, carbon, space)
  }

  // What a replica holds: its notes, and what became of each op in the shared order.
  fn state(space: &Space) -> Value {
    let ops = oplog::entries(space.store()).unwrap();
    let results: Vec<Value> = ops.iter().map(|op| json!([op.key, results::of(space.store(), &op.hash).unwrap()])).collect();
    json!({"notes": texts(&space.store().reader().unwrap()).unwrap(), "results": results})
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
    assert_eq!(settled["notes"], expected, "{name}: in order");
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
        vec![script(10, json!([{"insert": "a"}])), script(20, json!([{"delete": "a"}])), script(30, json!([{"replace": ["a", "b"]}]))],
        json!([]),
      ),
      (
        "an edit refused before the note arrived applies once it has",
        vec![script(10, json!([{"insert": "a"}])), script(20, json!([{"replace": ["a", "b"]}]))],
        json!(["b"]),
      ),
      (
        "a read sees every op before it, whenever they arrived",
        vec![script(10, json!([{"insert": "a"}])), script(20, json!([{"insert": "b"}, {"read": true}])), script(30, json!([{"read": true}]))],
        json!(["a", "b"]),
      ),
      (
        "a value kept unique by a query, never by a constraint, is inserted only by the first op",
        vec![script(10, json!([{"unless": "a", "do": [{"insert": "a"}]}])), script(20, json!([{"unless": "a", "do": [{"insert": "a"}]}]))],
        json!(["a"]),
      ),
      (
        "an insert before an edit of another row, which the edit looked up by its key",
        vec![
          script(5, json!([{"set": ["k", "first"]}])),
          script(10, json!([{"insert": "a"}])),
          script(20, json!([{"set": ["k", "second"]}])),
        ],
        json!(["a", "second"]),
      ),
      (
        "an edit of a row by its key before the insert of that row",
        vec![script(10, json!([{"set": ["k", "edited"]}])), script(20, json!([{"set": ["k", "inserted"]}]))],
        json!(["inserted"]),
      ),
      (
        "inserts of different rows into one table",
        vec![script(10, json!([{"insert": "a"}])), script(20, json!([{"insert": "b"}])), script(30, json!([{"insert": "c"}]))],
        json!(["a", "b", "c"]),
      ),
    ];
    for (name, ops, expected) in scenarios {
      converges(name, &ops, expected);
    }
  }

  #[test]
  fn a_watch_shows_the_state_a_rewind_settles_on() {
    let (_directory, carbon) = device();
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(20, json!([{"replace": ["a", "b"]}]))));
    receive(&carbon, row(&carbon, &script(10, json!([{"insert": "a"}]))));
    assert_eq!(seen.borrow().last(), Some(&json!(["b"])));
  }

  #[test]
  fn a_late_insert_of_another_row_applies_in_place_and_nothing_re_runs() {
    let (_directory, carbon) = device();
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(20, json!([{"insert": "b"}]))));
    receive(&carbon, row(&carbon, &script(10, json!([{"insert": "a"}]))));
    // A rewind would have shown the later insert undone, then applied again.
    assert_eq!(*seen.borrow(), vec![json!([]), json!(["b"]), json!(["a", "b"])]);
  }

  #[test]
  fn a_late_insert_applies_in_place_before_an_edit_that_looked_up_another_row_by_its_key() {
    let (_directory, carbon) = device();
    let seen = Rc::new(std::cell::RefCell::new(vec![]));
    let seeing = Rc::clone(&seen);
    carbon.watch(ROOM, None, "GET [notes]/texts", Rc::new(move |texts| seeing.borrow_mut().push(texts.unwrap()))).unwrap();
    receive(&carbon, row(&carbon, &script(5, json!([{"set": ["k", "first"]}]))));
    receive(&carbon, row(&carbon, &script(20, json!([{"set": ["k", "second"]}]))));
    receive(&carbon, row(&carbon, &script(10, json!([{"insert": "a"}]))));
    // A rewind would have shown the edit undone, then made again.
    assert_eq!(*seen.borrow(), vec![json!([]), json!(["first"]), json!(["second"]), json!(["a", "second"])]);
  }

  #[test]
  fn a_call_made_after_receiving_an_op_sorts_after_it() {
    let (_directory, carbon) = device();
    // A minute ahead of this device's clock, within how far clocks drift.
    let future = crate::ops::clock::wall() + 60_000;
    receive(&carbon, row(&carbon, &script(future, json!([{"insert": "a"}]))));
    let space = space_now(&carbon, ROOM).unwrap();
    call_now(&carbon, &space, &Actor { identity: Some("alice".into()), device: Some("pixel".into()) }, "POST [notes]/run", json!({"do": [{"read": true}]})).unwrap();
    let ops = oplog::entries(space.store()).unwrap();
    assert!(ops[1].time > future);
    assert_eq!(results::of(space.store(), &ops[1].hash).unwrap(), Some(Outcome::Applied { output: json!([["a"]]), calls: json!([]) }));
  }

  #[test]
  fn a_row_whose_time_is_too_far_ahead_or_impossible_is_refused_unlogged_and_leaves_the_clock_alone() {
    let (_directory, carbon) = device();
    let wall = crate::ops::clock::wall();
    let cases = [("too far ahead, for now", wall + 3_600_000, 425), ("never", crate::ops::clock::NEVER, 400)];
    for (name, time, status) in cases {
      let refused = now(|received| carbon.receive(ROOM, row(&carbon, &script(time, json!([{"insert": "a"}]))), received));
      assert_eq!(refused.unwrap_err().status, status, "{name}");
    }
    let space = space_now(&carbon, ROOM).unwrap();
    assert_eq!(oplog::entries(space.store()).unwrap().len(), 0, "nothing was logged");
    let next = carbon.stamp(&Actor::default(), &Call::from_key("POST [notes]/run").unwrap(), &json!({}), "notes").time;
    assert!(next < wall + 60_000, "the clock never took in a refused time");
  }
}
