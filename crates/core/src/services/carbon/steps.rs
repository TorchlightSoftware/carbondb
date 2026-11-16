//! Setting something up, as `[@carbon]` does: a list of steps run in order,
//! each once the one before it has answered, stopping at the first that
//! fails. A step is an effect, or a call in a space or channel by its id, as
//! an actor; both are effects of `[@carbon]`'s own op.

use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;

use crate::error::Result;
use crate::service_call::ServiceCall;
use crate::{Actor, Call, HandlerContext, Routing};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
  // An effect, such as creating a channel's space.
  Effect { name: &'static str, input: Value },
  // A call, in a space or a channel by its id, as an actor.
  Call { space: String, actor: Actor, key: &'static str, input: Value },
}

// What the steps returned, in order, or the first error.
pub type Done = Box<dyn FnOnce(Result<Vec<Value>>)>;

pub fn run(call: Rc<ServiceCall>, steps: VecDeque<Step>, done: Done) {
  next(call, steps, vec![], done)
}

fn next(call: Rc<ServiceCall>, mut steps: VecDeque<Step>, mut returned: Vec<Value>, done: Done) {
  let Some(step) = steps.pop_front() else { return done(Ok(returned)) };
  let then = Rc::clone(&call);
  let answered: crate::Reply = Box::new(move |result| match result {
    Ok(value) => {
      returned.push(value);
      next(then, steps, returned, done)
    }
    Err(error) => done(Err(error)),
  });
  match step {
    Step::Effect { name, input } => HandlerContext::effect(call, name, input, answered),
    Step::Call { space, actor, key, input } => match Call::from_key(key) {
      Ok(key) => call.call_in(&space, actor, key, input, answered),
      Err(error) => answered(Err(error)),
    },
  }
}
