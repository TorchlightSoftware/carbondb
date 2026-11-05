//! Test helpers shared across the core's modules: a CarbonDB on a fresh
//! directory with a scripted test service, an identity on it, and a way to make
//! a call and read its answer at once.
//!
//! `[notes]` is the test service. `POST [notes]/run` and `GET [notes]/run` run
//! the script in their input, step by step, so a test states what a handler
//! does as data:
//!
//!   {"insert": "a"}                                insert a note, its id drawn from the op
//!   {"read": true}                                 add the texts this handler sees to the output
//!   {"call": "POST [notes]/run", "input": {..}}    make a call, adding its answer to the output
//!   {"call": .., "catch": true}                    the same, adding a failure's status instead of failing
//!   {"fail": "why"}                                fail with a 409

use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{Value, json};

use crate::error::{Result, conflict};
use crate::{Actor, Call, Carbon, Db, Identity, Invocation, Service, Settled, Space, now};

// A single-node CarbonDB in a temporary directory, which lives as long as the
// directory, running `[notes]`.
pub fn carbon() -> (tempfile::TempDir, Rc<Carbon>) {
  let directory = tempfile::tempdir().unwrap();
  let config = directory.path().join("carbondb.toml");
  std::fs::write(&config, "system = \"data/system.db\"\n[sync]\nmode = \"none\"\n").unwrap();
  let carbon = Carbon::open(&config).unwrap();
  register_notes(&carbon);
  (directory, carbon)
}

pub fn register_notes(carbon: &Carbon) {
  let migrations = vec!["CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT NOT NULL)".to_string()];
  carbon.services().register(Service::replicated("notes", migrations, Rc::new(notes))).unwrap();
}

// Make a call and return its answer, which every service here gives at once.
pub fn call_now(carbon: &Rc<Carbon>, space: &Rc<Space>, actor: &Actor, key: &str, input: Value) -> Result<Settled> {
  now(|reply| carbon.submit(space, actor.clone(), Call::from_key(key).unwrap(), input, reply))
}

pub fn alice(carbon: &Rc<Carbon>) -> Identity {
  let input = json!({"slug": "alice", "device": "Pixel9", "services": ["notes"]});
  let created = call_now(carbon, carbon.system(), &Actor::default(), "POST [@carbon]/identity", input).unwrap();
  serde_json::from_value(created.value).unwrap()
}

// An identity acting from its first device.
pub fn actor(identity: &Identity) -> Actor {
  Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) }
}

// A space running `[notes]`, nested in the identity's personal space.
pub fn room(carbon: &Rc<Carbon>, identity: &Identity) -> Rc<Space> {
  let personal = carbon.space(&identity.personal).unwrap();
  let created = call_now(carbon, &personal, &actor(identity), "POST [@carbon]/space", json!({"name": "Room", "services": ["notes"]})).unwrap();
  carbon.space(created.value["space"].as_str().unwrap()).unwrap()
}

pub fn texts(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM note ORDER BY text", &[])?;
  Ok(Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect()))
}

fn notes(invocation: Rc<dyn Invocation>) {
  match invocation.call().resource.as_str() {
    "/run" => {
      let script = invocation.input()["do"].as_array().cloned().unwrap_or_default();
      step(invocation, VecDeque::from(script), vec![])
    }
    "/texts" => {
      let result = texts(invocation.as_ref());
      let _ = invocation.settle(result);
    }
    _ => {
      let _ = invocation.settle(Err(invocation.call().unknown()));
    }
  }
}

// Run the rest of a script, then settle with what it gathered.
fn step(invocation: Rc<dyn Invocation>, mut script: VecDeque<Value>, mut output: Vec<Value>) {
  let Some(next) = script.pop_front() else {
    let _ = invocation.settle(Ok(json!(output)));
    return;
  };
  let done = if let Some(text) = next.get("insert") {
    invocation.draw().and_then(|id| invocation.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(id), text.clone()])).map(|_| ())
  } else if next.get("read").is_some() {
    texts(invocation.as_ref()).map(|seen| output.push(seen))
  } else if let Some(why) = next["fail"].as_str() {
    Err(conflict(why))
  } else {
    let call = Call::from_key(next["call"].as_str().unwrap()).unwrap();
    let catch = next["catch"] == json!(true);
    let caller = Rc::clone(&invocation);
    return invocation.send(call, next["input"].clone(), Box::new(move |result| match result {
      Ok(value) => {
        output.push(value);
        step(caller, script, output)
      }
      Err(error) if catch => {
        output.push(json!({"error": error.status}));
        step(caller, script, output)
      }
      Err(error) => {
        let _ = caller.settle(Err(error));
      }
    }));
  };
  match done {
    Ok(()) => step(invocation, script, output),
    Err(error) => {
      let _ = invocation.settle(Err(error));
    }
  }
}
