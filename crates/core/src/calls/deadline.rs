//! A call's deadline: once its space is ready, whatever the call waits on (the
//! ops ahead of it in the log, its own handler) it settles within its
//! service's timeout, or with a 504. Its space's startup is bounded on its
//! own (`crate::channels::lifecycle`), so the deadline begins when the call is taken by
//! a ready space, or when the space it waits on becomes ready. It follows the
//! call: while the call waits, passing answers its caller and lets go of it
//! where it waits; once it runs, passing expires it, rolling an op back.
//! Answering the call cancels it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use crate::carbon::{Cancel, Carbon};
use crate::error::{ServiceError, timed_out};
use crate::calls::service_call::ServiceCall;

pub(crate) struct Deadline {
  key: String,
  timeout: Duration,
  stage: RefCell<Stage>,
  begun: Cell<bool>,
  cancel: RefCell<Option<Cancel>>,
}

// Where the call is when the deadline passes.
enum Stage {
  // Waiting to run: what lets go of it there and answers its caller.
  Waiting(Box<dyn FnOnce(ServiceError)>),
  Running(Rc<ServiceCall>),
  Answered,
}

impl Deadline {
  // A deadline for the call `key`, passing `timeout` after it begins.
  pub(crate) fn new(key: String, timeout: Duration) -> Rc<Deadline> {
    Rc::new(Deadline { key, timeout, stage: RefCell::new(Stage::Answered), begun: Cell::new(false), cancel: RefCell::default() })
  }

  // A deadline begun now, for a call no space start delays.
  pub(crate) fn arm(carbon: &Carbon, key: String, timeout: Duration) -> Rc<Deadline> {
    let deadline = Deadline::new(key, timeout);
    deadline.begin(carbon);
    deadline
  }

  // Begin counting, once: the call's space is ready.
  pub(crate) fn begin(self: &Rc<Self>, carbon: &Carbon) {
    if self.begun.replace(true) {
      return;
    }
    let passing = Rc::clone(self);
    self.cancel.replace(Some(carbon.after(self.timeout, Box::new(move || passing.pass()))));
  }

  // The call waits: when the deadline passes first, `give_up` lets go of it and answers its caller.
  pub(crate) fn waiting(&self, give_up: Box<dyn FnOnce(ServiceError)>) {
    self.stage.replace(Stage::Waiting(give_up));
  }

  // The call runs: when the deadline passes first, it expires.
  pub(crate) fn running(&self, call: &Rc<ServiceCall>) {
    self.stage.replace(Stage::Running(Rc::clone(call)));
  }

  // The call is answered: the deadline no longer passes.
  pub(crate) fn answered(&self) {
    self.stage.replace(Stage::Answered);
    if let Some(cancel) = self.cancel.take() {
      cancel();
    }
  }

  fn pass(&self) {
    self.cancel.take();
    match self.stage.replace(Stage::Answered) {
      Stage::Waiting(give_up) => give_up(timed_out(&self.key, self.timeout)),
      Stage::Running(call) => call.expire(self.timeout),
      Stage::Answered => {}
    }
  }
}
