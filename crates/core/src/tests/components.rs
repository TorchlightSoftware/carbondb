//! Components: every service but `[@carbon]` runs on the WASM host. `[probe]`
//! is a fixture component that reports what it sees of its host.

use super::*;
use crate::testing::{NOTES_TIMEOUT, PROBE, add, afresh, later, probed, until};
use crate::wasm::{self, Package};
use carbondb_service_bindings::manifest::sources;
use std::path::Path;


fn probe(carbon: &Rc<crate::Carbon>, identity: &crate::Identity, key: &str, input: Value) -> crate::Result<Value> {
  on(carbon, &identity.personal, identity, key, input)
}

#[test]
fn every_committed_component_was_built_from_its_sources_as_they_are() {
  let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
  let bindings = [root.join("crates/service-bindings/src"), root.join("crates/service-bindings/wit")];
  for (name, directory) in [("channel", "builtins"), ("identity", "builtins"), ("probe", "fixtures")] {
    let roots: Vec<_> = std::iter::once(root.join("crates/services").join(name)).chain(bindings.iter().cloned()).collect();
    let built = std::fs::read_to_string(root.join("crates/core").join(directory).join(format!("{name}.source"))).unwrap();
    assert_eq!(built.trim(), sources(&roots).unwrap(), "{name} is stale: run scripts/build-components");
  }
}

#[test]
fn calls_to_a_service_still_compiling_wait_for_it_and_answer_once_it_has() {
  let (_directory, carbon, _timers) = crate::testing::timed();
  let identity = alice(&carbon);
  let package = Package::read(&afresh(PROBE)).unwrap();
  let added = later(|added| carbon.add("probe", package, added));
  on(&carbon, &identity.personal, &identity, "POST [channel]/services", json!({"name": "probe"})).unwrap();
  let call = |key: &'static str, input: Value| later(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), key, input, answer));
  let (wrote, read) = (call("POST [probe]/note", json!({"text": "early"})), call("GET [probe]/notes", Value::Null));
  on(&carbon, &identity.personal, &identity, "POST [notes]/run", json!({"do": [{"insert": "meanwhile"}]})).unwrap();
  until(|| [added.borrow().is_some(), wrote.borrow().is_some(), read.borrow().is_some()] == [true; 3]);
  assert!(added.borrow_mut().take().unwrap().is_ok());
  assert!(wrote.borrow_mut().take().unwrap().is_ok(), "the write waited in its space's log");
  assert!(read.borrow_mut().take().unwrap().is_ok(), "the read was held, not refused");
  assert_eq!(probe(&carbon, &identity, "GET [probe]/notes", Value::Null).unwrap(), json!([{"text": "early"}]));
}

#[test]
fn a_package_that_does_not_compile_is_refused_once_it_has_tried_and_its_name_left_free() {
  let (_directory, carbon, _timers) = crate::testing::timed();
  let broken = crate::wasm::package(b"\0asm\x0d\x00\x01\x00\x7f\x01\x00", "name = \"broken\"\n").unwrap();
  let refused = add(&carbon, "broken", &broken).unwrap_err();
  assert_eq!((refused.status, refused.message.starts_with("Not a component this host can run")), (400, true), "{refused:?}");
  assert!(carbon.services().find("broken").is_none());
}

#[test]
fn a_package_is_a_component_carrying_its_service_toml_and_versioned_by_its_bytes() {
  let package = Package::read(PROBE).unwrap();
  assert_eq!((package.manifest.name.as_str(), package.manifest.timeout), ("probe", Some(100)));
  assert_eq!(package.version, crate::hash::of_bytes(PROBE));
  let refused = [("not a component", b"plain text".to_vec()), ("a component without a service.toml", b"\0asm\x0d\x00\x01\x00".to_vec())];
  for (why, bytes) in refused {
    assert_eq!(Package::read(&bytes).err().map(|error| error.status), Some(400), "{why}");
  }
}

#[test]
fn a_component_sets_up_its_tables_and_reads_and_writes_them_through_its_binding() {
  let (_directory, carbon, _timers, identity) = probed();
  for text in ["b", "a"] {
    probe(&carbon, &identity, "POST [probe]/note", json!({"text": text})).unwrap();
  }
  assert_eq!(probe(&carbon, &identity, "GET [probe]/notes", Value::Null).unwrap(), json!([{"text": "a"}, {"text": "b"}]));
  assert_eq!(wasm::running(), 0, "nothing is left running once calls settle");
}

#[test]
fn a_write_sees_its_ops_time_and_randomness_and_never_draws_the_same_id_twice() {
  let (_directory, carbon, _timers, identity) = probed();
  let seen = probe(&carbon, &identity, "POST [probe]/seen", json!({"inner": true})).unwrap();
  let (outer, inner) = (&seen["outer"], &seen["inner"]);
  let op = oplog::entries(service(&carbon, &identity.personal, "probe").store()).unwrap().pop().unwrap();
  assert_eq!((&outer["clock"], &inner["clock"]), (&json!(op.time), &json!(op.time)), "every clock reads the op's time");
  assert_eq!(outer["random"], inner["random"], "randomness is drawn from the op, the same in every instance");
  let ids = [&outer["first"], &inner["first"], &seen["last"]].map(|id| id.as_str().unwrap().to_string());
  assert_eq!(ids.iter().collect::<std::collections::BTreeSet<_>>().len(), 3, "draws continue across a call inside the op: {ids:?}");
  let again = probe(&carbon, &identity, "POST [probe]/seen", json!({})).unwrap();
  assert_ne!(again["random"], outer["random"], "another op draws other randomness");
}

#[test]
fn a_get_sees_this_devices_clock_and_draws_nothing() {
  let (_directory, carbon, _timers, identity) = probed();
  let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
  let seen = probe(&carbon, &identity, "GET [probe]/seen", json!({})).unwrap();
  assert!(seen["clock"].as_u64().unwrap() >= before);
  assert_eq!(seen["first"], Value::Null);
}

#[test]
fn a_trap_refuses_its_op_and_the_next_call_runs_in_a_fresh_instance() {
  let (_directory, carbon, _timers, identity) = probed();
  let error = probe(&carbon, &identity, "POST [probe]/trap", json!({})).unwrap_err();
  assert_eq!(error.status, 500);
  assert_eq!(outcomes(&service(&carbon, &identity.personal, "probe")).last(), Some(&Err(500)));
  probe(&carbon, &identity, "POST [probe]/note", json!({"text": "after"})).unwrap();
  assert_eq!(probe(&carbon, &identity, "GET [probe]/notes", Value::Null).unwrap(), json!([{"text": "after"}]));
}

#[test]
fn a_component_calls_inside_its_op_and_messages_another_service_and_a_loop_back_is_refused() {
  let (_directory, carbon, _timers, identity) = probed();
  let inside = json!({"key": "POST [probe]/note", "input": {"text": "inside"}});
  probe(&carbon, &identity, "POST [probe]/call", inside).unwrap();
  let message = json!({"key": "POST [notes]/run", "input": {"do": [{"insert": "n"}, {"read": true}]}});
  assert_eq!(probe(&carbon, &identity, "POST [probe]/call", message).unwrap(), json!([["n"]]));
  let back = json!({"key": "POST [notes]/run", "input": {"do": [{"call": "POST [probe]/note", "input": {"text": "x"}, "catch": true}]}});
  assert_eq!(probe(&carbon, &identity, "POST [probe]/call", back).unwrap(), json!([{"error": 508}]));
  assert_eq!(probe(&carbon, &identity, "GET [probe]/notes", Value::Null).unwrap(), json!([{"text": "inside"}]));
}

#[test]
fn a_call_that_outlives_its_deadline_abandons_its_instance() {
  let (_directory, carbon, timers, identity) = probed();
  let waiting = json!({"key": "GET [notes]/hang", "input": null});
  let answer = later(|reply| carbon.call(Some(&identity.personal), Some(&identity.id), "POST [probe]/call", waiting, reply));
  assert!(answer.borrow().is_none());
  assert_eq!(wasm::running(), 1);
  assert!(timers.delays().contains(&std::time::Duration::from_millis(100)), "the probe's own timeout, beside [notes]' {NOTES_TIMEOUT:?}");
  timers.run();
  assert_eq!(answer.borrow_mut().take().unwrap().unwrap_err().status, 504);
  assert_eq!(wasm::running(), 0, "the instance was dropped with its call");
}
