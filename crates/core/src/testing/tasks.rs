//! The host's timers and tasks as a test drives them: the core's timers run
//! when the test says, its tasks whenever the test drives them, and a task
//! woken from another thread, as one waiting on the network is, once the test
//! waits for its answer.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context as Poll, Wake, Waker};
use std::time::Duration;

use crate::carbon::{Schedule, Spawn, Task};
use crate::error::{Result, internal};

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

// Start something that answers through a callback, and return where its
// answer lands, for a test to read once it has answered.
pub fn later<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Rc<RefCell<Option<Result<T>>>> {
  let answer = Rc::new(RefCell::new(None));
  let slot = Rc::clone(&answer);
  start(Box::new(move |result| *slot.borrow_mut() = Some(result)));
  drive();
  answer
}

// Start something that answers through a callback, and return its answer,
// which every call here gives once its tasks have run.
pub fn now<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(Result<T>)>)) -> Result<T> {
  later(start).borrow_mut().take().unwrap_or_else(|| Err(internal("The call answered later")))
}
