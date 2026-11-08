//! Setting something up, as `[@carbon]` does: a list of steps run in order,
//! each once the one before it has applied, stopping at the first that fails.
//! A step is a call, made in the space `[@carbon]` runs in as a sub-call it
//! carries, or in another space as an op there; or host work on this device.

use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;

use crate::error::Result;
use crate::service_call::ServiceCall;
use crate::space::Space;
use crate::{Actor, Call, Routing};

// Where a step's call runs: the space `[@carbon]` was called in, or another by id.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
  Here,
  Space(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
  // A new space's database, on this device.
  Create(String),
  // A call: a sub-call here, or an op in another space.
  Call { target: Target, actor: Actor, key: &'static str, input: Value },
  // Set up the services a space's origin named, once that origin applies.
  SetUp(String, Vec<String>),
}

// What the steps' calls returned, in order, or the first error.
pub type Done = Box<dyn FnOnce(Result<Vec<Value>>)>;

pub fn run(call: Rc<ServiceCall>, steps: VecDeque<Step>, done: Done) {
  next(call, steps, vec![], done)
}

fn next(call: Rc<ServiceCall>, mut steps: VecDeque<Step>, mut returned: Vec<Value>, done: Done) {
  let Some(step) = steps.pop_front() else { return done(Ok(returned)) };
  let carbon = Rc::clone(call.carbon());
  match step {
    Step::Create(id) => match carbon.create_space(&id) {
      Ok(_) => next(call, steps, returned, done),
      Err(error) => done(Err(error)),
    },
    Step::SetUp(id, services) => {
      let then = Rc::clone(&call);
      carbon.clone().with_space(&id, Box::new(move |space| match space.and_then(|space| carbon.set_up(&space, &services)) {
        Ok(()) => next(then, steps, returned, done),
        Err(error) => done(Err(error)),
      }))
    }
    Step::Call { target, actor, key, input } => {
      let made = Rc::clone(&call);
      in_space(&call, target, Box::new(move |space| {
        let (space, key) = match space.and_then(|space| Ok((space, Call::from_key(key)?))) {
          Ok(found) => found,
          Err(error) => return done(Err(error)),
        };
        let then = Rc::clone(&made);
        made.call_in(&space, actor, key, input, Box::new(move |result| match result {
          Ok(value) => {
            returned.push(value);
            next(then, steps, returned, done)
          }
          Err(error) => done(Err(error)),
        }))
      }))
    }
  }
}

fn in_space(call: &Rc<ServiceCall>, target: Target, then: Box<dyn FnOnce(Result<Rc<Space>>)>) {
  match target {
    Target::Here => then(Ok(Rc::clone(call.space()))),
    Target::Space(id) => Rc::clone(call.carbon()).with_space(&id, then),
  }
}
