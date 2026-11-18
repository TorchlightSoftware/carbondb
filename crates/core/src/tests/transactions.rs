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
