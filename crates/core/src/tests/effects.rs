use super::*;
use crate::ops::effects::Made;
use crate::testing::{register_script, register_script_making};

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
fn late(carbon: &Rc<crate::Carbon>, space: &crate::ServiceSpace, script: Value, effects: Option<Vec<Made>>) {
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
fn a_write_back_into_a_service_waiting_on_the_call_is_refused_at_once_and_a_read_is_answered() {
  let (_directory, carbon, identity, room) = room_with_jots(&[]);
  let back = |via: Value| json!({"do": [{"insert": "n"}, {"call": "POST [jots]/run", "input": {"do": via}}]});
  let cases = [
    ("a write straight back", json!([{"call": "POST [notes]/run", "input": {"do": []}, "catch": true}]), json!([[{"error": 508}]])),
    ("a write back through a third service", json!([{"call": "POST [loud]/run", "input": {"do": [{"call": "POST [notes]/run", "input": {"do": []}, "catch": true}]}}]), json!([[[{"error": 508}]]])),
    ("a read back, which never waits and sees what is committed, not the calling op's own write", json!([{"call": "GET [notes]/texts"}]), json!([[["n", "n"]]])),
  ];
  for (why, via, expected) in cases {
    let answered = on(&carbon, &room, &identity, "POST [notes]/run", back(via)).unwrap();
    assert_eq!(answered, expected, "{why}");
  }
}

#[test]
fn a_refused_founding_leaves_no_channel_or_service_behind() {
  let (directory, carbon) = carbon();
  let setup = Rc::new(|db: Rc<dyn Db>| db.run("CREATE TABLE IF NOT EXISTS hit (id INTEGER PRIMARY KEY)", &[]).map(|_| ()));
  carbon.register(crate::Service::replicated("counter", "counter-v1", Some("db"), crate::testing::NOTES_TIMEOUT, setup, Rc::new(|_| {}))).unwrap();
  let input = json!({"slug": "alice", "device": "Pixel9", "services": ["counter"]});
  let refused = call_now(&carbon, carbon.system(), &Actor::default(), "POST [@carbon]/identity", input).unwrap_err();
  assert_eq!(refused.details, Some(json!({"table": "hit"})));
  let files: Vec<String> = std::fs::read_dir(directory.path().join("data")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().to_string()).filter(|name| name.ends_with(".db")).collect();
  assert_eq!(files, ["system.db"]);
  let held = carbon.system().store().reader().unwrap().all("SELECT id FROM carbon_system_space", &[]).unwrap();
  assert_eq!(held, Vec::<Value>::new());
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
