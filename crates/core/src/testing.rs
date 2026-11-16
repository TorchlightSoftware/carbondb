//! Test helpers shared across the core's modules: a CarbonDB on a fresh
//! directory with a scripted test service, an identity on it, and a way to make
//! a call and read its answer at once.
//!
//! `[notes]` is the test service. `POST [notes]/run` and `GET [notes]/run` run
//! the script in their input, step by step, so a test states what a handler
//! does as data:
//!
//!   {"insert": "a"}                                insert a note, its id drawn from the op
//!   {"replace": ["a", "b"]}                        change note a's text to b, or fail with a 404
//!   {"set": ["k", "b"]}                            set the text of the note keyed k to b, looking it up
//!                                                  by its key and inserting it when there is none
//!   {"delete": "a"}                                delete note a, or fail with a 404
//!   {"unless": "a", "do": [..]}                    run the steps in `do` only when no note says a
//!   {"read": true}                                 add the texts this handler sees to the output
//!   {"call": "POST [notes]/run", "input": {..}}    make a call, adding its answer to the output
//!   {"call": .., "catch": true}                    the same, adding a failure's status instead of failing
//!   {"fail": "why"}                                fail with a 409
//!   {"hang": true}                                 never settle, keeping the call for `hung`
//!
//! `GET [notes]/hang` never settles either. `[notes]` must settle within
//! `NOTES_TIMEOUT`; the host's timers are `Timers`, and a test runs what the
//! core scheduled when it says. The host's tasks, such as a component's call,
//! run on `Tasks`, which a test drives until none is ready; `now` and `later`
//! drive them, as does running the timers. A task woken from another thread,
//! as one waiting on the network is, runs once the test waits for its answer
//! with `awaited`. `Outside` is the world outside: an HTTP server of the
//! test's own.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context as Poll, Wake, Waker};
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Result, conflict, internal, not_found, unknown};
use crate::carbon::{Schedule, Spawn, Task};
use crate::{Actor, Call, Carbon, Db, HandlerContext, Identity, Routing, Service, Settled, Space};

// A single-node CarbonDB in a temporary directory, which lives as long as the
// directory, running `[notes]`.
pub fn carbon() -> (tempfile::TempDir, Rc<Carbon>) {
  let (directory, carbon, _timers) = timed();
  (directory, carbon)
}

// A test's config: single-node, its compiled components kept where every
// test's are, so a run compiles each component once.
pub fn config() -> String {
  let compiled = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/carbondb-compiled");
  format!("system = \"data/system.db\"\ncompiled = {:?}\n[sync]\nmode = \"none\"\n", compiled.display().to_string())
}

// The core's test fixture `[probe]`, a Rust component that reports what it sees of its host.
pub const PROBE: &[u8] = include_bytes!("../fixtures/probe.wasm");

// A package's bytes with a custom section of its own appended, so its version
// is new and this process compiles it afresh.
pub fn afresh(bytes: &[u8]) -> Vec<u8> {
  static FRESH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
  let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
  let name = format!("carbondb:test:{since}:{}", FRESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
  [bytes, &[0, name.len() as u8 + 1, name.len() as u8], name.as_bytes()].concat()
}

// Register a package under `name`, once it has compiled.
pub fn add(carbon: &Rc<Carbon>, name: &str, bytes: &[u8]) -> Result<()> {
  let package = crate::wasm::Package::read(bytes)?;
  awaited(|added| carbon.add(name, package, added))
}

// A personal channel running `[probe]` beside `[notes]`, with the timers.
pub fn probed() -> (tempfile::TempDir, Rc<Carbon>, Timers, Identity) {
  let (directory, carbon, timers) = timed();
  add(&carbon, "probe", PROBE).unwrap();
  let identity = alice(&carbon);
  on(&carbon, &identity.personal, &identity, "POST [channel]/services", json!({"name": "probe"})).unwrap();
  (directory, carbon, timers, identity)
}

// The same, with the timers it schedules on.
pub fn timed() -> (tempfile::TempDir, Rc<Carbon>, Timers) {
  let directory = tempfile::tempdir().unwrap();
  std::fs::write(directory.path().join("carbondb.toml"), config()).unwrap();
  let timers = Timers::default();
  let carbon = Carbon::open(&directory.path().join("carbondb.toml"), timers.schedule(), tasks()).unwrap();
  register_notes(&carbon);
  (directory, carbon, timers)
}

// The CarbonDB in a directory again, as after a restart, without `[notes]`.
pub fn reopen(directory: &tempfile::TempDir) -> Rc<Carbon> {
  Carbon::open(&directory.path().join("carbondb.toml"), Timers::default().schedule(), tasks()).unwrap()
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

  // Run what was scheduled to run within `delay`, as though that much time had
  // passed, and the tasks it set going; what that schedules waits.
  pub fn pass(&self, delay: Duration) {
    let due: Vec<usize> = self.scheduled.borrow().iter().filter(|(_, (scheduled, _))| *scheduled <= delay).map(|(id, _)| *id).collect();
    let due: Vec<Scheduled> = due.into_iter().filter_map(|id| self.scheduled.borrow_mut().remove(&id)).collect();
    for (_, then) in due {
      then();
    }
    drive();
  }

  // Run everything scheduled so far, and the tasks it set going.
  pub fn run(&self) {
    let due = self.scheduled.take();
    for (_, then) in due.into_values() {
      then();
    }
    drive();
  }
}

// The host's tasks as a test drives them: each runs when the test drives
// them, and again whenever it is woken, until none is ready.
#[derive(Default)]
struct Tasks {
  running: RefCell<BTreeMap<usize, Task>>,
  ready: Arc<Queue>,
  next: Cell<usize>,
}

// The tasks ready to run, and a signal for a test waiting on a wake from another thread.
#[derive(Default)]
struct Queue {
  ready: Mutex<VecDeque<usize>>,
  woken: Condvar,
}

impl Queue {
  fn push(&self, task: usize) {
    self.ready.lock().unwrap().push_back(task);
    self.woken.notify_all();
  }

  fn pop(&self) -> Option<usize> {
    self.ready.lock().unwrap().pop_front()
  }

  // Wait until a task is ready, or the time passes.
  fn wait(&self, longest: Duration) {
    let ready = self.ready.lock().unwrap();
    let _ = self.woken.wait_timeout_while(ready, longest, |ready| ready.is_empty()).unwrap();
  }
}

// Wakes a task by queueing it as ready.
struct Ready {
  task: usize,
  ready: Arc<Queue>,
}

impl Wake for Ready {
  fn wake(self: Arc<Self>) {
    self.ready.push(self.task);
  }
}

thread_local! {
  // Each test's tasks, on the thread it runs on.
  static TASKS: Rc<Tasks> = Rc::default();
}

// The host's way to run a task, as the core receives it.
pub fn tasks() -> Spawn {
  Rc::new(|task| {
    let (tasks, id) = TASKS.with(|tasks| {
      let id = tasks.next.replace(tasks.next.get() + 1);
      tasks.running.borrow_mut().insert(id, task);
      tasks.ready.push(id);
      (Rc::clone(tasks), id)
    });
    Box::new(move || {
      tasks.running.borrow_mut().remove(&id);
    })
  })
}

// Run the ready tasks until none is ready.
pub fn drive() {
  let tasks = TASKS.with(Rc::clone);
  loop {
    let Some(id) = tasks.ready.pop() else { return };
    let Some(mut task) = tasks.running.borrow_mut().remove(&id) else { continue };
    let waker = Waker::from(Arc::new(Ready { task: id, ready: Arc::clone(&tasks.ready) }));
    if task.as_mut().poll(&mut Poll::from_waker(&waker)).is_pending() {
      tasks.running.borrow_mut().insert(id, task);
    }
  }
}

// Drive the tasks until the answer lands, waiting for those woken from other
// threads, as a task waiting on the network is.
pub fn awaited<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Result<T> {
  let answer = later(start);
  until(|| answer.borrow().is_some());
  answer.borrow_mut().take().expect("answered")
}

// Drive the tasks until something holds, waiting for those woken from other threads.
pub fn until(holds: impl Fn() -> bool) {
  const LONGEST: Duration = Duration::from_secs(10);
  let since = std::time::Instant::now();
  drive();
  while !holds() {
    assert!(since.elapsed() < LONGEST, "never came to hold");
    TASKS.with(|tasks| tasks.ready.wait(LONGEST));
    drive();
  }
}

// The world outside, as the test plays it: an HTTP server on this machine
// that answers `/slow` after a pause, `/echo` with the body it was sent, and
// any other path with its own name, and keeps what it was asked.
pub struct Outside {
  address: String,
  asked: Arc<Mutex<Vec<String>>>,
}

// How long `/slow` pauses before it answers.
pub const SLOW: Duration = Duration::from_millis(300);

impl Outside {
  pub fn start() -> Outside {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let asked = Arc::new(Mutex::new(vec![]));
    let keeping = Arc::clone(&asked);
    std::thread::spawn(move || {
      for stream in listener.incoming().flatten() {
        let keeping = Arc::clone(&keeping);
        std::thread::spawn(move || answer(stream, &keeping));
      }
    });
    Outside { address, asked }
  }

  pub fn url(&self, path: &str) -> String {
    format!("http://{}{path}", self.address)
  }

  // What it was asked so far: each request's method and path.
  pub fn asked(&self) -> Vec<String> {
    self.asked.lock().unwrap().clone()
  }
}

// The address of a server that hangs up on every connection, before saying anything.
pub fn hanging_up() -> String {
  let listener = TcpListener::bind("127.0.0.1:0").unwrap();
  let address = listener.local_addr().unwrap().to_string();
  std::thread::spawn(move || listener.incoming().for_each(drop));
  address
}

// Read one request and answer it.
fn answer(stream: TcpStream, asked: &Mutex<Vec<String>>) {
  let mut reader = BufReader::new(stream);
  let mut line = String::new();
  reader.read_line(&mut line).unwrap();
  let [method, path] = [0, 1].map(|at| line.split_whitespace().nth(at).unwrap_or_default().to_string());
  let mut length = 0;
  loop {
    let mut header = String::new();
    reader.read_line(&mut header).unwrap();
    if header.trim().is_empty() {
      break;
    }
    if let Some((name, value)) = header.split_once(':') && name.eq_ignore_ascii_case("content-length") {
      length = value.trim().parse().unwrap();
    }
  }
  let mut body = vec![0; length];
  reader.read_exact(&mut body).unwrap();
  asked.lock().unwrap().push(format!("{method} {path}"));
  let answered = match path.as_str() {
    "/slow" => {
      std::thread::sleep(SLOW);
      b"slow".to_vec()
    }
    "/echo" => body,
    other => other.trim_start_matches('/').as_bytes().to_vec(),
  };
  let mut stream = reader.into_inner();
  let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", answered.len());
  stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&answered)).unwrap();
}

// `[notes]`'s timeout, unlike the default, so a test sees its own set a call's deadline.
pub const NOTES_TIMEOUT: Duration = Duration::from_millis(100);

// Start something that answers through a callback, and return where its
// answer lands, for a test to read once it has answered.
pub fn later<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Rc<RefCell<Option<Result<T>>>> {
  let answer = Rc::new(RefCell::new(None));
  let slot = Rc::clone(&answer);
  start(Box::new(move |result| *slot.borrow_mut() = Some(result)));
  drive();
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
  register_script_making(carbon, name, &[]);
}

// The scripted test service, declaring the effects it makes again on every run.
pub fn register_script_making(carbon: &Rc<Carbon>, name: &str, always: &[&str]) {
  let setup = Rc::new(|db: Rc<dyn Db>| db.run("CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID", &[]).map(|_| ()));
  let service = Service::replicated(name, &format!("{name}-v1"), Some("db"), NOTES_TIMEOUT, setup, Rc::new(notes));
  carbon.register(Service { always: always.iter().map(|effect| effect.to_string()).collect(), ..service }).unwrap();
}

// Start something that answers through a callback, and return its answer,
// which every call here gives once its tasks have run.
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

// A call on a channel's handle, as the identity, answered at once.
pub fn on(carbon: &Rc<Carbon>, channel: &str, identity: &Identity, key: &str, input: Value) -> Result<Value> {
  now(|answer| carbon.call(Some(channel), Some(&identity.id), key, input, answer))
}

// The space a channel names for `[name]`.
pub fn service(carbon: &Rc<Carbon>, channel: &str, name: &str) -> Rc<Space> {
  carbon.service_space(channel, &Call::from_key(&format!("GET [{name}]/any")).unwrap()).unwrap()
}

// The identity's personal channel's `[notes]`.
pub fn personal_notes(carbon: &Rc<Carbon>, identity: &Identity) -> Rc<Space> {
  service(carbon, &identity.personal, "notes")
}

// A channel "Room" in the identity's personal channel, running `[notes]`: its id.
pub fn room_channel(carbon: &Rc<Carbon>, identity: &Identity) -> String {
  let created = on(carbon, &identity.personal, identity, "POST [channel]/channel", json!({"name": "Room"})).unwrap();
  let room = created["channel"].as_str().unwrap().to_string();
  on(carbon, &room, identity, "POST [channel]/services", json!({"name": "notes"})).unwrap();
  room
}

// The `[notes]` of a fresh "Room" channel in the identity's personal channel.
pub fn room(carbon: &Rc<Carbon>, identity: &Identity) -> Rc<Space> {
  let room = room_channel(carbon, identity);
  service(carbon, &room, "notes")
}

pub fn texts(db: &dyn Db) -> Result<Value> {
  let rows = db.all("SELECT text FROM note ORDER BY text", &[])?;
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

// A step that changes a note, which fails when there is no such note.
fn changed(rows: Result<u64>, text: &Value) -> Result<()> {
  match rows? {
    0 => Err(not_found(format!("No note {text}"))),
    _ => Ok(()),
  }
}

// Set a note's text by its key, as an edit looks up the one row it changes.
fn set(db: &dyn Db, key: &Value, text: &Value) -> Result<()> {
  let found = db.all("SELECT id FROM note WHERE id = ?", std::slice::from_ref(key))?;
  let sql = if found.is_empty() { "INSERT INTO note (text, id) VALUES (?, ?)" } else { "UPDATE note SET text = ? WHERE id = ?" };
  db.run(sql, &[text.clone(), key.clone()]).map(|_| ())
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
  } else if let Some([from, to]) = next["replace"].as_array().map(Vec::as_slice) {
    changed(context.run("UPDATE note SET text = ? WHERE text = ?", &[to.clone(), from.clone()]), from)
  } else if let Some([key, text]) = next["set"].as_array().map(Vec::as_slice) {
    set(context.as_ref(), key, text)
  } else if let Some(text) = next.get("delete") {
    changed(context.run("DELETE FROM note WHERE text = ?", std::slice::from_ref(text)), text)
  } else if let Some(text) = next.get("unless") {
    let steps = next["do"].as_array().cloned().unwrap_or_default();
    seen(context.as_ref()).map(|texts| {
      if !texts.as_array().is_some_and(|texts| texts.contains(text)) {
        steps.into_iter().rev().for_each(|step| script.push_front(step));
      }
    })
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
