//! Causes: an op a `callsync` write made is bound to the op that made it,
//! within one channel, and follows it on every replica: undone when its cause
//! is undone, and never published unless its cause succeeded. A write made
//! through `call` is bound to nothing, and stays.

use super::*;
use crate::testing::{awaited, register_script, replicated, until};

// A room running `[notes]` and `[jots]`, and alice.
fn room_with_jots() -> (tempfile::TempDir, Rc<crate::Carbon>, crate::Identity, String) {
  let (directory, carbon) = carbon();
  register_script(&carbon, "jots");
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  on(&carbon, &room, &identity, "POST [@carbon]/services", json!({"name": "jots"})).unwrap();
  (directory, carbon, identity, room)
}

#[test]
fn a_call_follows_its_cause_when_a_late_op_refuses_the_cause_at_its_place() {
  let (_directory, carbon, identity, room) = room_with_jots();
  let (notes, jots) = (service(&carbon, &room, "notes"), service(&carbon, &room, "jots"));
  arrives(&carbon, &notes, json!([{"insert": "allowed"}]), 1);
  // Sally closes the books, which posts to [jots], only while she is allowed.
  let close = json!({"do": [{"replace": ["allowed", "allowed"]}, {"callsync": "POST [jots]/run", "input": {"do": [{"insert": "closed"}]}}]});
  on(&carbon, &room, &identity, "POST [notes]/run", close).unwrap();
  assert_eq!(texts(&jots.store().reader().unwrap()).unwrap(), json!(["closed"]));
  let posted = &entries(&jots)[0];
  assert_eq!(oplog::row(jots.channel.log(), &posted.hash).unwrap().unwrap().cause, Some(entries(&notes)[1].hash.clone()), "the post is bound to the close that made it");

  // Bob's revocation, made before she closed, arrives late.
  arrives(&carbon, &notes, json!([{"delete": "allowed"}]), 2);
  assert_eq!(outcomes(&notes), vec![Ok(()), Ok(()), Err(404)], "her close is refused at its place");
  assert_eq!(texts(&jots.store().reader().unwrap()).unwrap(), json!([]), "and the post it made is undone with it");
  assert_eq!(outcomes(&jots), vec![Err(409)]);
}

#[test]
fn the_caller_decides_what_a_failed_call_means() {
  let (_directory, carbon, identity, room) = room_with_jots();
  let (notes, jots) = (service(&carbon, &room, "notes"), service(&carbon, &room, "jots"));
  let tolerated = json!({"do": [{"insert": "closed"}, {"call": "POST [jots]/run", "input": {"do": [{"fail": "no chat"}]}, "catch": true}]});
  assert_eq!(on(&carbon, &room, &identity, "POST [notes]/run", tolerated).unwrap(), json!([{"error": 409}]));
  assert_eq!(texts(&notes.store().reader().unwrap()).unwrap(), json!(["closed"]), "a call that fails without failing its caller leaves the caller applied");
  assert_eq!(outcomes(&jots), vec![Err(409)]);
}

#[test]
fn an_op_refused_on_its_first_run_undoes_its_calls_and_none_of_them_is_published() {
  let (_directory, carbon) = replicated();
  register_script(&carbon, "jots");
  let identity = crate::testing::found(&carbon, "alice", &["notes", "jots"]);
  let called = awaited(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), "POST [notes]/run", json!({"do": [{"callsync": "POST [jots]/run", "input": {"do": [{"insert": "j"}]}}, {"fail": "no"}]}), answer));
  assert_eq!(called.unwrap_err().status, 409);
  let jots = service(&carbon, &identity.personal, "jots");
  until(|| outcomes(&jots) == vec![Err(409)]);
  assert_eq!(texts(&jots.store().reader().unwrap()).unwrap(), json!([]));
  let post = entries(&jots)[0].hash.clone();
  let published = || crate::ops::published::unpublished(jots.channel.log(), &identity.device).unwrap().iter().any(|row| row.op.hash == post);
  assert!(!published(), "a call whose cause failed is not publishable");
  let sent = jots.channel.log().reader().unwrap().get("SELECT 1 AS sent FROM carbon_published WHERE hash = ?", &[json!(post)]).unwrap();
  assert!(sent.is_null(), "and was never published");
}

#[test]
fn a_call_into_another_channel_carries_no_cause() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let personal = carbon.channel(&identity.personal).unwrap();
  let rows = oplog::rows(personal.log()).unwrap();
  let caused = |key: &str| rows.iter().filter(|row| row.op.key == key).map(|row| row.cause.is_some()).collect::<Vec<bool>>();
  assert_eq!(caused("POST [@carbon]/services"), [false], "founding called [@carbon] here from the system, another channel");
  assert_eq!(caused("POST [channel]/services"), [true], "[@carbon] here named the service under its own op");
}

#[test]
fn a_write_through_call_stays_when_its_caller_is_refused_and_one_through_callsync_is_undone() {
  let cases = [("call", json!(["closed"]), vec![Ok(())]), ("callsync", json!([]), vec![Err(409)])];
  for (binding, left, posted) in cases {
    let (_directory, carbon, identity, room) = room_with_jots();
    let (notes, jots) = (service(&carbon, &room, "notes"), service(&carbon, &room, "jots"));
    arrives(&carbon, &notes, json!([{"insert": "allowed"}]), 1);
    let close = json!({"do": [{"replace": ["allowed", "allowed"]}, {binding: "POST [jots]/run", "input": {"do": [{"insert": "closed"}]}}]});
    on(&carbon, &room, &identity, "POST [notes]/run", close).unwrap();
    arrives(&carbon, &notes, json!([{"delete": "allowed"}]), 2);
    assert_eq!(outcomes(&notes), vec![Ok(()), Ok(()), Err(404)], "{binding}: the close is refused at its place");
    assert_eq!((texts(&jots.store().reader().unwrap()).unwrap(), outcomes(&jots)), (left, posted), "{binding}");
  }
}
