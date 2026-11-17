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
fn a_get_reads_committed_state_while_a_write_is_still_running() {
  let (_directory, carbon, _timers) = timed();
  let identity = alice(&carbon);
  let personal = personal_notes(&carbon, &identity);
  hung();
  let write = later(|reply| carbon.submit(&personal, actor(&identity), crate::Call::from_key("POST [notes]/run").unwrap(), json!({"do": [{"insert": "a"}, {"hang": true}]}), reply));
  assert_eq!(call_now(&carbon, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap().value, json!([]));
  let [writing] = hung().try_into().ok().unwrap();
  writing.settle(Ok(json!("done"))).unwrap();
  assert!(write.borrow_mut().take().unwrap().is_ok());
  assert_eq!(call_now(&carbon, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap().value, json!(["a"]));
}

#[test]
fn a_call_that_settles_cancels_its_deadline() {
  let (_directory, carbon, timers) = timed();
  let identity = alice(&carbon);
  let personal = personal_notes(&carbon, &identity);
  assert!(timers.delays().is_empty(), "creating an identity left no deadline behind");
  call_now(&carbon, &personal, &actor(&identity), "POST [notes]/run", json!({"do": [{"insert": "a"}]})).unwrap();
  call_now(&carbon, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap();
  assert!(timers.delays().is_empty());
  assert_eq!(texts(personal.store()).unwrap(), json!(["a"]));
}
