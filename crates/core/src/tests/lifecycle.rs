//! A space's lifecycle: it starts (its package, its setup, its health check)
//! before it applies anything, holding its calls meanwhile, and ends ready or
//! stopped with why. Starting is bounded by its service's startup timeout; a
//! call's own deadline begins once its space is ready.

use super::*;
use crate::testing::{Timers, later, timed};
use std::cell::Cell;
use std::time::Duration;

// How long a call to [gauge] may take, longer than a health check's retry.
const TIMEOUT: Duration = Duration::from_millis(1000);
const RETRY: Duration = Duration::from_millis(100);

// `[gauge]`: notes, a GET that never settles, and a health check that
// passes once the test says it is healthy.
fn gauge(healthy: Rc<Cell<bool>>, startup: Duration) -> crate::Service {
  let setup = Rc::new(|db: Rc<dyn Db>| db.run("CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID", &[]).map(|_| ()));
  let entry = Rc::new(move |context: crate::testing::Context| {
    let result = match (context.method(), context.resource()) {
      (_, "/hang") => return,
      (_, "/health") if healthy.get() => Ok(json!("ok")),
      (_, "/health") => Err(crate::error::system_failure("not yet")),
      ("POST", "/notes") => context.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(context.op().map(|op| op.id())), context.input()["text"].clone()]).map(|_| Value::Null),
      _ => texts(context.as_ref()),
    };
    let _ = context.settle(result);
  });
  let service = crate::Service::replicated("gauge", "gauge-v1", Some("db"), TIMEOUT, setup, entry);
  crate::Service { health: Some("/health".into()), startup, ..service }
}

// A room naming [gauge], which starts as the room names it.
fn room_with_gauge(startup: Duration) -> (tempfile::TempDir, Rc<crate::Carbon>, Timers, crate::Identity, String, Rc<Cell<bool>>) {
  let (directory, carbon, timers) = timed();
  let healthy = Rc::new(Cell::new(false));
  carbon.register(gauge(Rc::clone(&healthy), startup)).unwrap();
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "gauge"})).unwrap();
  (directory, carbon, timers, identity, room, healthy)
}

fn gauge_state(carbon: &Rc<crate::Carbon>, room: &str, identity: &crate::Identity) -> Value {
  on(carbon, room, identity, "GET [@carbon]/status", json!({"service": "gauge"})).unwrap()
}

#[test]
fn calls_wait_out_a_start_longer_than_their_own_timeout_and_are_answered_once_it_is_ready() {
  let (_directory, carbon, timers, identity, room, healthy) = room_with_gauge(Duration::from_secs(10));
  assert_eq!(gauge_state(&carbon, &room, &identity)["state"], json!("initializing"));
  let call = |key: &'static str, input: Value| later(|answer| carbon.call(Some(&room), Some(&identity.id), key, input, answer));
  let (wrote, read) = (call("POST [gauge]/notes", json!({"text": "early"})), call("GET [gauge]/notes", Value::Null));
  timers.pass(TIMEOUT);
  assert!(wrote.borrow().is_none() && read.borrow().is_none(), "starting does not count against a call's own timeout");

  healthy.set(true);
  timers.pass(RETRY);
  assert_eq!(gauge_state(&carbon, &room, &identity)["state"], json!("idle"));
  assert_eq!(wrote.borrow_mut().take().unwrap().unwrap(), Value::Null);
  assert_eq!(read.borrow_mut().take().unwrap().unwrap(), json!(["early"]), "the write waiting in its log applied first");
}

#[test]
fn a_call_held_while_its_space_starts_has_its_whole_timeout_once_the_space_is_ready() {
  let (_directory, carbon, timers, identity, room, healthy) = room_with_gauge(Duration::from_secs(10));
  let hanging = later(|answer| carbon.call(Some(&room), Some(&identity.id), "GET [gauge]/hang", Value::Null, answer));
  healthy.set(true);
  timers.pass(RETRY);
  assert!(hanging.borrow().is_none(), "it runs once the space is ready, and hangs");
  timers.pass(TIMEOUT);
  let error = hanging.borrow_mut().take().unwrap().unwrap_err();
  assert_eq!((error.status, error.code.as_str()), (504, "TIMED_OUT"));
}

#[test]
fn a_space_not_ready_within_its_startup_timeout_stops_and_its_waiting_calls_fail_with_why() {
  let startup = Duration::from_millis(3000);
  let (_directory, carbon, timers, identity, room, _healthy) = room_with_gauge(startup);
  let waiting = later(|answer| carbon.call(Some(&room), Some(&identity.id), "POST [gauge]/notes", json!({"text": "lost"}), answer));
  timers.pass(startup);
  let state = gauge_state(&carbon, &room, &identity);
  assert_eq!((&state["state"], &state["error"]["code"], &state["error"]["details"]["failure"]["message"]), (&json!("stopped"), &json!("STARTUP_TIMEOUT"), &json!("not yet")));
  let failed = waiting.borrow_mut().take().unwrap().unwrap_err();
  assert_eq!((failed.status, failed.code.as_str(), failed.details.unwrap()["failure"]["code"].clone()), (503, "SERVICE_STOPPED", json!("STARTUP_TIMEOUT")));
}
