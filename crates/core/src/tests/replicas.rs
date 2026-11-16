//! Replicas: the same ops delivered in different orders converge on the same
//! state, as though each had applied them in the shared order.

use super::*;
use crate::ops::op::stamp;
use crate::ops::oplog::LogRow;
use crate::channels::service::Runs;
use crate::testing::now;
use crate::{Call, Carbon, ServiceSpace};

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
fn replica(rows: &[&LogRow]) -> (tempfile::TempDir, Rc<Carbon>, Rc<ServiceSpace>) {
  let (directory, carbon) = device();
  rows.iter().for_each(|row| receive(&carbon, (*row).clone()));
  let space = space_now(&carbon, ROOM).unwrap();
  (directory, carbon, space)
}

// What a replica holds: its notes, and what became of each op in the shared order.
fn state(space: &ServiceSpace) -> Value {
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
