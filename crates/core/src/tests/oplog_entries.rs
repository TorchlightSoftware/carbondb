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
  assert!(matches!(space.run_state(), crate::RunState::Stopped { error } if error.code == "PACKAGE_UNAVAILABLE"), "this device lacks its package");
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
