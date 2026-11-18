use super::*;
use crate::RunState;
use std::cell::RefCell;

// Every RunState a space moves to, by its name.
fn states_of(carbon: &Rc<crate::Carbon>, space: &crate::ServiceStore) -> Rc<RefCell<Vec<String>>> {
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
  assert_eq!((error.status, error.code.as_str()), (503, "SERVICE_STOPPED"));
  assert_eq!(error.details.unwrap()["failure"]["code"], json!("SYSTEM_FAILURE"));
  assert!(matches!(space.run_state(), RunState::Stopped { .. }));
  for key in ["POST [notes]/run", "GET [notes]/texts"] {
    assert_eq!(call_now(&carbon, &space, &actor(&identity), key, insert("b")).unwrap_err().code, "SERVICE_STOPPED", "{key}");
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
  crate::testing::left(&space, op.clone());
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
fn submitted(carbon: &Rc<crate::Carbon>, space: &Rc<crate::ServiceStore>, actor: &crate::Actor, key: &str, input: Value) -> Rc<RefCell<Option<Result<Value, u16>>>> {
  let answer = Rc::new(RefCell::new(None));
  let into = Rc::clone(&answer);
  let call = crate::Call::from_key(key).unwrap();
  carbon.submit(space, actor.clone(), call, input, Box::new(move |settled| *into.borrow_mut() = Some(settled.map(|settled| settled.value).map_err(|error| error.status))));
  answer
}

// A space whose package this device lacks cannot get it single-node, so it
// stops at once, refusing calls with why; registering the package later
// initializes it again.
#[test]
fn a_space_whose_package_this_device_lacks_stops_until_the_package_is_registered() {
  let (directory, carbon) = carbon();
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  drop(carbon);
  let restarted = crate::testing::reopen(&directory);
  let space = service(&restarted, &room, "notes");
  let refused = now(|reply| restarted.submit(&space, actor(&identity), crate::Call::from_key("POST [notes]/run").unwrap(), insert("a"), reply)).unwrap_err();
  assert!(refused.message.starts_with("[notes] stopped, and applies nothing until it is resumed"), "named as calls name it: {}", refused.message);
  assert_eq!((refused.status, refused.code.as_str(), refused.details.unwrap()["failure"]["code"].clone()), (503, "SERVICE_STOPPED", json!("PACKAGE_UNAVAILABLE")));
  assert_eq!(entries(&space), vec![], "nothing is logged for a stopped space");

  crate::testing::register_notes(&restarted);
  assert_eq!(space.run_state(), crate::RunState::Idle);
  assert_eq!(submitted(&restarted, &space, &actor(&identity), "POST [notes]/run", insert("b")).borrow_mut().take(), Some(Ok(json!([]))));
  let elsewhere = submitted(&restarted, &space, &actor(&identity), "POST [elsewhere]/run", insert("c"));
  assert_eq!(elsewhere.borrow_mut().take(), Some(Err(404)), "a space answers only its own service");
}
