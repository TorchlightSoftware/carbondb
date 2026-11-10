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
//!   {"hang": true}                                 never settle, keeping the call for `hung`
//!
//! `GET [notes]/hang` never settles either. `[notes]` must settle within
//! `NOTES_TIMEOUT`; the host's timers are `Timers`, and a test runs what the
//! core scheduled when it says.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Result, conflict, internal, unknown};
use crate::carbon::Schedule;
use crate::{Actor, Call, Carbon, Db, HandlerContext, Identity, Routing, Service, Settled, Space};

// A single-node CarbonDB in a temporary directory, which lives as long as the
// directory, running `[notes]`.
pub fn carbon() -> (tempfile::TempDir, Rc<Carbon>) {
  let (directory, carbon, _timers) = timed();
  (directory, carbon)
}

// The same, with the timers it schedules on.
pub fn timed() -> (tempfile::TempDir, Rc<Carbon>, Timers) {
  let directory = tempfile::tempdir().unwrap();
  std::fs::write(directory.path().join("carbondb.toml"), "system = \"data/system.db\"\n[sync]\nmode = \"none\"\n").unwrap();
  let timers = Timers::default();
  let carbon = Carbon::open(&directory.path().join("carbondb.toml"), timers.schedule()).unwrap();
  register_notes(&carbon);
  (directory, carbon, timers)
}

// The CarbonDB in a directory again, as after a restart, without `[notes]`.
pub fn reopen(directory: &tempfile::TempDir) -> Rc<Carbon> {
  Carbon::open(&directory.path().join("carbondb.toml"), Timers::default().schedule()).unwrap()
}

// The host's timers as a test drives them: what the core scheduled and has
// not cancelled, kept until the test runs it as though its delay had passed.
#[derive(Default, Clone)]
pub struct Timers {
  scheduled: Rc<RefCell<BTreeMap<usize, Scheduled>>>,
  next: Rc<Cell<usize>>,
}

// What the core asked to run, and after how long.
type Scheduled = (Duration, Box<dyn FnOnce()>);

impl Timers {
  pub fn schedule(&self) -> Schedule {
    let timers = self.clone();
    Rc::new(move |delay, then| {
      let id = timers.next.replace(timers.next.get() + 1);
      timers.scheduled.borrow_mut().insert(id, (delay, then));
      let scheduled = Rc::clone(&timers.scheduled);
      Box::new(move || {
        scheduled.borrow_mut().remove(&id);
      })
    })
  }

  // The delays of what is scheduled and has neither run nor been cancelled.
  pub fn delays(&self) -> Vec<Duration> {
    self.scheduled.borrow().values().map(|(delay, _)| *delay).collect()
  }

  // Run everything scheduled so far.
  pub fn run(&self) {
    let due = self.scheduled.take();
    for (_, then) in due.into_values() {
      then();
    }
  }
}

// `[notes]`'s timeout, unlike the default, so a test sees its own set a call's deadline.
pub const NOTES_TIMEOUT: Duration = Duration::from_millis(100);

// Start something that answers through a callback, and return where its
// answer lands, for a test to read once it has answered.
pub fn later<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Rc<RefCell<Option<Result<T>>>> {
  let answer = Rc::new(RefCell::new(None));
  let slot = Rc::clone(&answer);
  start(Box::new(move |result| *slot.borrow_mut() = Some(result)));
  answer
}

thread_local! {
  // The calls `[notes]` left hanging, for a test to settle late.
  static HUNG: RefCell<Vec<Context>> = RefCell::default();
}

// The calls `[notes]` has left hanging since this was last asked.
pub fn hung() -> Vec<Context> {
  HUNG.with(|hung| hung.take())
}

fn hang(context: Context) {
  HUNG.with(|hung| hung.borrow_mut().push(context));
}

pub fn register_notes(carbon: &Rc<Carbon>) {
  register_script(carbon, "notes");
}

// The scripted test service under a name of its own, with its own notes.
pub fn register_script(carbon: &Rc<Carbon>, name: &str) {
  let init = Rc::new(|db: Rc<dyn Db>| db.run("CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID", &[]).map(|_| ()));
  carbon.register(Service::replicated(name, &format!("{name}-v1"), Some("db"), NOTES_TIMEOUT, init, Rc::new(notes))).unwrap();
}

// Start something that answers through a callback, and return its answer,
// which every call here gives at once.
pub fn now<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Result<T> {
  later(start).borrow_mut().take().unwrap_or_else(|| Err(internal("The call answered later")))
}

pub fn call_now(carbon: &Rc<Carbon>, space: &Rc<Space>, actor: &Actor, key: &str, input: Value) -> Result<Settled> {
  now(|reply| carbon.submit(space, actor.clone(), Call::from_key(key).unwrap(), input, reply))
}

pub fn space_now(carbon: &Rc<Carbon>, id: &str) -> Result<Rc<Space>> {
  now(|opened| carbon.with_space(id, opened))
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
  let personal = space_now(carbon, &identity.personal).unwrap();
  let created = call_now(carbon, &personal, &actor(identity), "POST [@carbon]/space", json!({"name": "Room", "services": ["notes"]})).unwrap();
  space_now(carbon, created.value["space"].as_str().unwrap()).unwrap()
}

pub fn texts(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM \"notes/db/note\" ORDER BY text", &[])?;
  Ok(Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect()))
}

pub type Context = Rc<dyn HandlerContext>;

fn notes(context: Context) {
  match context.resource() {
    "/run" => {
      let script = context.input()["do"].as_array().cloned().unwrap_or_default();
      step(context, VecDeque::from(script), vec![])
    }
    "/texts" => {
      let result = seen(context.as_ref());
      let _ = context.settle(result);
    }
    "/hang" => hang(context),
    resource => {
      let _ = context.settle(Err(unknown(context.method(), resource)));
    }
  }
}

// The texts a handler sees, by the plain name its service gives the table.
fn seen(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM note ORDER BY text", &[])?;
  Ok(Value::Array(rows.into_iter().map(|row| row["text"].clone()).collect()))
}

// Run the rest of a script, then settle with what it gathered.
fn step(context: Context, mut script: VecDeque<Value>, mut output: Vec<Value>) {
  let Some(next) = script.pop_front() else {
    let _ = context.settle(Ok(json!(output)));
    return;
  };
  let done = if let Some(text) = next.get("insert") {
    // A GET has no op to draw an id from, and its `run` refuses before the id matters.
    let id = context.op().map(|op| op.id()).unwrap_or_default();
    context.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(id), text.clone()]).map(|_| ())
  } else if next.get("read").is_some() {
    seen(context.as_ref()).map(|texts| output.push(texts))
  } else if let Some(why) = next["fail"].as_str() {
    Err(conflict(why))
  } else if next.get("hang").is_some() {
    return hang(context);
  } else {
    let catch = next["catch"] == json!(true);
    let caller = Rc::clone(&context);
    let key = next["call"].as_str().unwrap().to_string();
    return context.call(&key, next["input"].clone(), Box::new(move |result| match result {
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
    Ok(()) => step(context, script, output),
    Err(error) => {
      let _ = context.settle(Err(error));
    }
  }
}
