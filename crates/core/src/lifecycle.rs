//! A space's lifecycle on this device. A space defines the service it runs
//! when it is created; before it applies anything here it starts, in three
//! steps:
//!
//! 1. **Its package.** It waits while this device compiles it. A package this
//!    device does not have cannot be fetched from peers single-node, so that
//!    step fails at once.
//! 2. **Its setup**, run on its binding.
//! 3. **Its health check**, when its service.toml names one: a GET it keeps
//!    trying until it answers without an error.
//!
//! Starting is bounded on its own: each space has its service's startup
//! timeout, from when it begins, to become ready. Then it is Idle, applying
//! its ops one at a time; or Stopped, with why, when a step fails or its time
//! runs out. While it starts, its ops wait in its log and its GETs are held;
//! their own deadlines begin once it is ready, so a slow start never eats
//! into them. Once it stops they fail with why. A package compiling or
//! registered later starts again the spaces that were stopped for want of it,
//! and resuming a stopped space that was never set up does too.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::carbon::{Cancel, Carbon};
use crate::deadline::Deadline;
use crate::dispatch::{Reply, Taken};
use crate::error::{Result, ServiceError};
use crate::services::{DEFAULT_STARTUP_TIMEOUT, Service};
use crate::space::{RunState, Space};
use crate::watch::WatchId;
use crate::{Actor, Call, Routing};

// How long a failing health check waits before it tries again.
const RETRY: Duration = Duration::from_millis(100);

// A GET held while its space starts, taken again once the space is ready or stopped.
pub(crate) type Held = Box<dyn FnOnce()>;

// A space starting: the last failure its health check met, and how to stop
// waiting for its time to run out.
pub(crate) struct Starting {
  last: RefCell<Option<ServiceError>>,
  running_out: RefCell<Option<Cancel>>,
}

impl Starting {
  fn end(&self) {
    if let Some(cancel) = self.running_out.take() {
      cancel();
    }
  }
}

impl Carbon {
  // Start a space: Initializing, its startup time running, then each step in
  // turn, stopping it when one fails. Returns the watches it ran again.
  pub(crate) fn initialize(self: &Rc<Self>, space: &Rc<Space>) -> Vec<WatchId> {
    let rerun = self.begin_starting(space);
    self.advance(space);
    rerun
  }

  // Mark a space Initializing and start its time running.
  pub(crate) fn begin_starting(self: &Rc<Self>, space: &Rc<Space>) -> Vec<WatchId> {
    let rerun = self.set_run_state(space, RunState::Initializing { services: vec![space.runs.package.clone()] });
    let within = self.services().find(&space.runs.package).map_or(DEFAULT_STARTUP_TIMEOUT, |service| service.startup);
    let starting = Rc::new(Starting { last: RefCell::default(), running_out: RefCell::default() });
    let (carbon, here, mine) = (Rc::clone(self), Rc::clone(space), Rc::clone(&starting));
    starting.running_out.replace(Some(self.after(within, Box::new(move || carbon.out_of_time(&here, &mine, within)))));
    if let Some(earlier) = self.starting_now(&space.id, Some(starting)) {
      earlier.end();
    }
    rerun
  }

  // Take the steps a space can take now, stopping it when one fails.
  fn advance(self: &Rc<Self>, space: &Rc<Space>) {
    if let Err(error) = self.prepare(space) {
      self.stop(space, error);
    }
  }

  // The steps that run at once: its package, unless it is still compiling,
  // which advances it again once it has; its setup; then its health check,
  // which runs on. Fails when this device lacks its package or its setup fails.
  pub(crate) fn prepare(self: &Rc<Self>, space: &Rc<Space>) -> Result<()> {
    let package = &space.runs.package;
    if self.services().is_compiling(package) {
      return Ok(());
    }
    let service = self.services().find(package).ok_or_else(|| unavailable(package))?;
    space.set_up(&service)?;
    self.check_health(space, &service);
    Ok(())
  }

  // A package has compiled or been registered: the open spaces waiting on it
  // take their next steps, and those stopped for want of it start again.
  pub(crate) fn package_ready(self: &Rc<Self>, package: &str) {
    for space in self.waiting_on(package) {
      match space.run_state() {
        RunState::Initializing { .. } => self.advance(&space),
        _ => {
          self.initialize(&space);
        }
      }
    }
  }

  // A package failed to compile: stop the spaces that wait for it, with why.
  pub(crate) fn package_failed(self: &Rc<Self>, package: &str, error: &ServiceError) {
    for space in self.waiting_on(package) {
      self.stop(&space, error.clone());
    }
  }

  // The open spaces running a package that are not set up yet.
  fn waiting_on(&self, package: &str) -> Vec<Rc<Space>> {
    self.open_spaces().into_iter().filter(|space| space.runs.package == package && space.service().is_none()).collect()
  }

  // Hold a GET while its space starts, until the space is ready or stopped.
  pub(crate) fn hold(self: &Rc<Self>, space: &Rc<Space>, taken: Taken) {
    let (carbon, here) = (Rc::clone(self), Rc::clone(space));
    self.keep_held(&space.id, Box::new(move || carbon.take(&here, taken)));
  }

  // The space is ready: its startup is over, the deadlines of the ops
  // waiting in its log begin, those ops apply, then the GETs held on it run.
  fn ready(self: &Rc<Self>, space: &Rc<Space>) {
    if let Some(starting) = self.starting_now(&space.id, None) {
      starting.end();
    }
    self.set_run_state(space, RunState::Idle);
    space.waiting_deadlines().iter().for_each(|deadline| deadline.begin(self));
    self.drain(space);
    self.release(&space.id);
  }

  // Its time ran out before it was ready: it stops, with the last failure its health check met.
  fn out_of_time(self: &Rc<Self>, space: &Rc<Space>, starting: &Rc<Starting>, within: Duration) {
    let current = self.starting_of(&space.id).is_some_and(|now| Rc::ptr_eq(&now, starting));
    if !current || !matches!(space.run_state(), RunState::Initializing { .. }) {
      return;
    }
    self.starting_now(&space.id, None);
    self.stop(space, not_started(&space.runs.name, within, starting.last.take()));
  }

  // Its health check: ready at once when its service names none.
  fn check_health(self: &Rc<Self>, space: &Rc<Space>, service: &Rc<Service>) {
    let Some(path) = &service.health else { return self.ready(space) };
    match Call::from_key(&format!("GET [{}]{path}", space.runs.name)) {
      Ok(call) => self.try_health(space, call, service.timeout),
      Err(error) => self.stop(space, error),
    }
  }

  // Try its health check once, as no one, under its service's timeout.
  fn try_health(self: &Rc<Self>, space: &Rc<Space>, call: Call, timeout: Duration) {
    let (carbon, here, again) = (Rc::clone(self), Rc::clone(space), call.clone());
    let reply: Reply = Box::new(move |result| carbon.health_answered(&here, again, timeout, result.map(|settled| settled.value)));
    let deadline = Deadline::arm(self, call.key(), timeout);
    self.read(space, Taken { actor: Actor::default(), call, input: Value::Null, awaiting: vec![], deadline, reply });
  }

  // It passed, and the space is ready; or it failed, and tries again while the space is still starting.
  fn health_answered(self: &Rc<Self>, space: &Rc<Space>, call: Call, timeout: Duration, result: Result<Value>) {
    if !matches!(space.run_state(), RunState::Initializing { .. }) {
      return;
    }
    match result {
      Ok(_) => self.ready(space),
      Err(error) => {
        if let Some(starting) = self.starting_of(&space.id) {
          starting.last.replace(Some(error));
        }
        let (carbon, here) = (Rc::clone(self), Rc::clone(space));
        let _ = self.after(RETRY, Box::new(move || {
          if matches!(here.run_state(), RunState::Initializing { .. }) {
            carbon.try_health(&here, call, timeout)
          }
        }));
      }
    }
  }
}

// Why a space stops when this device does not have its package, and,
// single-node, has no peers to fetch it from.
pub fn unavailable(package: &str) -> ServiceError {
  ServiceError::new(503, "PACKAGE_UNAVAILABLE", format!("The package [{package}] is not on this device, and no peer can provide it; register it with services.add"))
    .with_details(json!({"service": package}))
}

// Why a space stops when it was not ready within its startup timeout.
fn not_started(name: &str, within: Duration, last: Option<ServiceError>) -> ServiceError {
  let why = last.as_ref().map_or(String::new(), |last| format!("; its health check last failed: {}", last.message));
  ServiceError::new(503, "STARTUP_TIMEOUT", format!("[{name}] did not start within {}ms{why}", within.as_millis())).with_details(json!({"failure": last}))
}
