//! A service reaches the outside world over wasi:http, every request the
//! `fetch` effect: made once in a write and replayed after, made every time in
//! a GET.

use super::*;
use crate::ops::effects::Made;
use crate::testing::{Outside, awaited, hanging_up, until};
use std::net::TcpListener;

// A personal channel running `[probe]`, which sends requests over wasi:http, and its space.
fn probed() -> (tempfile::TempDir, Rc<crate::Carbon>, crate::Identity, Rc<crate::ServiceSpace>) {
  let (directory, carbon, _timers, identity) = crate::testing::probed();
  let probe = service(&carbon, &identity.personal, "probe");
  (directory, carbon, identity, probe)
}

// `[probe]` sending these requests, all at once, in a write or a GET.
fn fetch(carbon: &Rc<crate::Carbon>, identity: &crate::Identity, method: &str, requests: Value) -> crate::Result<Value> {
  let key = format!("{method} [probe]/fetch");
  awaited(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), &key, json!({"requests": requests}), answer))
}

// An op from alice's other device arriving late, before everything here,
// with the effects its first run recorded, once every op has run again: a
// probe that reads the notes its fetches are noted beside rewinds and runs
// again.
fn late(carbon: &Rc<crate::Carbon>, probe: &crate::ServiceSpace, key: &str, input: Value, effects: Vec<Made>) {
  let actor = Actor { identity: Some("alice".into()), device: Some("laptop".into()) };
  let op = crate::ops::op::stamp(&actor, &crate::Call::from_key(key).unwrap(), &input, &carbon.services().version(&probe.runs.package), 1);
  awaited(|received| carbon.receive(&probe.id, oplog::LogRow { op, effects: Some(effects) }, received)).unwrap();
  until(|| !outcomes(probe).contains(&Err(0)));
}

// What the last op made of its run, and the fetches it recorded.
fn last(probe: &crate::ServiceSpace) -> (Value, Vec<Made>) {
  let op = oplog::entries(probe.store()).unwrap().pop().unwrap();
  let Some(Outcome::Applied { output, .. }) = results::of(probe.store(), &op.hash).unwrap() else { panic!("applied") };
  (output, oplog::effects(probe.store(), &op.hash).unwrap().unwrap_or_default())
}

fn early_note(carbon: &Rc<crate::Carbon>, probe: &crate::ServiceSpace) {
  late(carbon, probe, "POST [probe]/note", json!({"text": "early"}), vec![]);
}

#[test]
fn a_write_fetches_once_and_its_replay_after_a_rewind_gets_the_recorded_response() {
  let (_directory, carbon, identity, probe) = probed();
  let outside = Outside::start();
  let answered = fetch(&carbon, &identity, "POST", json!([{"url": outside.url("/echo"), "method": "POST", "body": "sent"}])).unwrap();
  assert_eq!(answered, json!([{"status": 200, "body": "sent"}]));
  let (_, recorded) = last(&probe);
  let fetched: Vec<(&str, Value, Value)> = recorded.iter().map(|made| (made.name.as_str(), made.input["body"].clone(), made.output.clone().unwrap()["body"].clone())).collect();
  assert_eq!(fetched, [("fetch", json!("c2VudA=="), json!("c2VudA=="))], "the whole request and response, its body written after it was sent");

  early_note(&carbon, &probe);
  assert_eq!(outcomes(&probe), vec![Ok(()), Ok(())], "the rewound op ran again at its place");
  assert_eq!(last(&probe).0, answered, "its replay got the recorded response");
  assert_eq!(outside.asked(), ["POST /echo"], "and sent nothing");
}

#[test]
fn requests_sent_at_once_are_recorded_in_the_order_sent_whatever_order_they_answer_in() {
  let (_directory, carbon, identity, probe) = probed();
  let outside = Outside::start();
  let answered = fetch(&carbon, &identity, "POST", json!([{"url": outside.url("/slow")}, {"url": outside.url("/fast")}])).unwrap();
  assert_eq!(answered, json!([{"status": 200, "body": "slow"}, {"status": 200, "body": "fast"}]));
  let bodies: Vec<Value> = last(&probe).1.iter().map(|made| made.output.clone().unwrap()["body"].clone()).collect();
  assert_eq!(bodies, [json!("c2xvdw=="), json!("ZmFzdA==")], "slow, then fast, though fast answered first");

  early_note(&carbon, &probe);
  assert_eq!(last(&probe).0, answered, "the replay hands each request its own response");
  assert_eq!(outside.asked().len(), 2);
}

#[test]
fn a_request_that_fails_is_recorded_failing_and_every_replay_meets_the_same_failure() {
  let (_directory, carbon, identity, probe) = probed();
  let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
  let answered = fetch(&carbon, &identity, "POST", json!([{"url": format!("http://{closed}/gone")}])).unwrap();
  assert!(answered[0]["error"].as_str().unwrap().contains("connection"), "{answered}");
  assert_eq!(last(&probe).1[0].output.clone().unwrap_err().code, "FETCH_FAILED");
  early_note(&carbon, &probe);
  assert_eq!(last(&probe).0, answered);
}

#[test]
fn a_replay_sending_a_request_its_first_run_never_made_finds_it_unreachable() {
  let (_directory, carbon, _identity, probe) = probed();
  let outside = Outside::start();
  late(&carbon, &probe, "POST [probe]/fetch", json!({"requests": [{"url": outside.url("/hello")}]}), vec![]);
  let (output, _) = last(&probe);
  assert!(output[0]["error"].as_str().unwrap().contains("cannot be reached from this device"), "{output}");
  assert_eq!(outside.asked(), Vec::<String>::new());
}

#[test]
fn an_https_request_goes_over_tls() {
  let (_directory, carbon, identity, _probe) = probed();
  let answered = fetch(&carbon, &identity, "GET", json!([{"url": format!("https://{}/", hanging_up())}])).unwrap();
  assert!(answered[0]["error"].as_str().unwrap().contains("TLS error"), "a server that hangs up fails the TLS handshake: {answered}");
}

// The bundled root certificates verify a real server's; it needs the internet.
#[test]
#[ignore = "reaches the internet"]
fn an_https_request_reaches_a_server_on_the_internet() {
  let (_directory, carbon, identity, _probe) = probed();
  let answered = fetch(&carbon, &identity, "GET", json!([{"url": "https://example.com/"}])).unwrap();
  assert_eq!(answered[0]["status"], json!(200), "{answered}");
  assert!(answered[0]["body"].as_str().unwrap().contains("Example Domain"));
}

#[test]
fn a_get_fetches_every_time_and_records_nothing() {
  let (_directory, carbon, identity, _probe) = probed();
  let outside = Outside::start();
  for _ in 0..2 {
    assert_eq!(fetch(&carbon, &identity, "GET", json!([{"url": outside.url("/hello")}])).unwrap(), json!([{"status": 200, "body": "hello"}]));
  }
  assert_eq!(outside.asked(), ["GET /hello", "GET /hello"]);
}
