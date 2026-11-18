//! Setting something up, as `[@carbon]` does: calls run in order, each once
//! the one before it has answered, stopping at the first that fails. Each is
//! a call in a space or channel by its id, as an actor, made as the `call`
//! effect of `[@carbon]`'s own op.

use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;

use crate::error::Result;
use crate::calls::service_call::ServiceCall;
use crate::{Actor, Call, Routing};

// A call in a space or a channel by its id, as an actor.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
  pub space: String,
  pub actor: Actor,
  pub key: &'static str,
  pub input: Value,
}

// What the steps returned, in order, or the first error.
pub type Done = Box<dyn FnOnce(Result<Vec<Value>>)>;

pub fn run(call: Rc<ServiceCall>, steps: VecDeque<Step>, done: Done) {
  next(call, steps, vec![], done)
}

fn next(call: Rc<ServiceCall>, mut steps: VecDeque<Step>, mut returned: Vec<Value>, done: Done) {
  let Some(Step { space, actor, key, input }) = steps.pop_front() else { return done(Ok(returned)) };
  let then = Rc::clone(&call);
  let answered: crate::Reply = Box::new(move |result| match result {
    Ok(value) => {
      returned.push(value);
      next(then, steps, returned, done)
    }
    Err(error) => done(Err(error)),
  });
  match Call::from_key(key) {
    Ok(key) => call.call_in(&space, actor, key, input, answered),
    Err(error) => answered(Err(error)),
  }
}
