//! Setting something up, as `[@carbon]` does: a list of steps run in order,
//! each once the one before it has applied, stopping at the first that fails.
//! A step is a call, made as an op of its own in a space, or host work on this
//! device.

use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;

use crate::error::Result;
use crate::frame::Frame;
use crate::{Actor, Call};

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
  // A call, as an op of its own in its space.
  Call { target: Target, actor: Actor, key: &'static str, input: Value },
  // Provision the services a space's origin named, once that origin applies.
  Provision(String, Vec<String>),
}

// What the steps' calls returned, in order, or the first error.
pub type Done = Box<dyn FnOnce(Result<Vec<Value>>)>;

pub fn run(frame: Rc<Frame>, steps: VecDeque<Step>, done: Done) {
  next(frame, steps, vec![], done)
}

fn next(frame: Rc<Frame>, mut steps: VecDeque<Step>, mut returned: Vec<Value>, done: Done) {
  let Some(step) = steps.pop_front() else { return done(Ok(returned)) };
  let carbon = Rc::clone(frame.carbon());
  let host = match step {
    Step::Create(id) => carbon.create_space(&id).map(|_| ()),
    Step::Provision(id, services) => carbon.space(&id).and_then(|space| carbon.provision(&space, &services)),
    Step::Call { target, actor, key, input } => {
      let space = match target {
        Target::Here => Ok(Rc::clone(frame.space())),
        Target::Space(id) => carbon.space(&id),
      };
      let (space, call) = match space.and_then(|space| Ok((space, Call::from_key(key)?))) {
        Ok(found) => found,
        Err(error) => return done(Err(error)),
      };
      let then = Rc::clone(&frame);
      return frame.call_in(&space, actor, call, input, Box::new(move |result| match result {
        Ok(value) => {
          returned.push(value);
          next(then, steps, returned, done)
        }
        Err(error) => done(Err(error)),
      }));
    }
  };
  match host {
    Ok(()) => next(frame, steps, returned, done),
    Err(error) => done(Err(error)),
  }
}
