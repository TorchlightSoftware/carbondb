//! Effects: what a service does outside its own tables, such as calling
//! another service or creating a space for a new one.
//!
//! Every replica re-runs each op's handler, but an effect happens once: on the
//! op's first run, on the device that made it. Its output, success or
//! failure, is recorded on the op's log row in the order the handler asked for
//! it, whatever order the outputs came back in, and travels with the op;
//! every later run, a replay on another device or after a rewind here, gets
//! the recorded outputs in order instead, even when the handler now asks for
//! something different. A replay asking for more effects than the first run
//! made gets an unreachable failure, the same on every replica. The core
//! records and hands back outputs without interpreting them: whether one is a
//! failure for the business is the service's decision.
//!
//! A service may declare an effect `replicate = "always"` in its service.toml;
//! such an effect is made again on every run on every replica, and never
//! recorded.
//!
//! The registry holds the handler for each effect by name. Single-node, this
//! device carries out what it can alone (`call`, `create`, `fetch`); an
//! effect that needs peers has no handler, and fails as unreachable.

use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::error::{Result, ServiceError};
use crate::service_call::ServiceCall;
use crate::Reply;

// An effect made on an op's first run: its name, its input and its output.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
  pub name: String,
  pub input: Value,
  pub output: Result<Value>,
}

// As it is kept: `{name, input, output}`, or `{name, input, error}` for a failure.
impl Serialize for Made {
  fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    let kept = match &self.output {
      Ok(output) => json!({"name": self.name, "input": self.input, "output": output}),
      Err(error) => json!({"name": self.name, "input": self.input, "error": error}),
    };
    kept.serialize(serializer)
  }
}

impl<'de> Deserialize<'de> for Made {
  fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Made, D::Error> {
    #[derive(Deserialize)]
    struct Kept {
      name: String,
      input: Value,
      output: Option<Value>,
      error: Option<ServiceError>,
    }
    let kept = Kept::deserialize(deserializer)?;
    let output = match kept.error {
      Some(error) => Err(error),
      None => Ok(kept.output.unwrap_or(Value::Null)),
    };
    Ok(Made { name: kept.name, input: kept.input, output })
  }
}

// The effects of one op's run: on a first run, those asked for so far, each
// in the slot it took when asked, holding its output once it came back; on a
// replay, the recorded ones still to hand back.
#[derive(Debug, Default)]
pub(crate) struct EffectLog {
  replaying: Option<VecDeque<Made>>,
  made: Vec<Made>,
}

// What an effect asked for gets on this run.
pub(crate) enum Turn {
  // Make it, keeping its output in this slot: this is the op's first run.
  Make(usize),
  // The output its first run recorded.
  Recorded(Result<Value>),
}

impl EffectLog {
  // The log of a run: a replay of the recorded effects, or a first run when none are recorded.
  pub(crate) fn of(recorded: Option<Vec<Made>>) -> EffectLog {
    EffectLog { replaying: recorded.map(VecDeque::from), made: vec![] }
  }

  // Whether this run records what it makes, being the op's first.
  pub(crate) fn first_run(&self) -> bool {
    self.replaying.is_none()
  }

  // What the next recorded effect gets: on a first run, the next slot, which
  // holds that the effect never came back until its output does; on a
  // replay, its recorded output, or an unreachable failure past the end of
  // the recording.
  pub(crate) fn next(&mut self, name: &str, input: &Value) -> Turn {
    match &mut self.replaying {
      None => {
        self.made.push(Made { name: name.to_string(), input: input.clone(), output: Err(unfinished(name)) });
        Turn::Make(self.made.len() - 1)
      }
      Some(recorded) => Turn::Recorded(recorded.pop_front().map_or_else(|| Err(beyond(name)), |made| made.output)),
    }
  }

  // What the effect asked for in this slot was made with, and its output, kept for its row.
  pub(crate) fn keep(&mut self, slot: usize, input: Value, output: Result<Value>) {
    self.made[slot] = Made { input, output, ..self.made[slot].clone() };
  }

  // What the first run made, in the order it was asked for, to write onto its row; none on a replay.
  pub(crate) fn made(&self) -> Option<&[Made]> {
    self.first_run().then_some(self.made.as_slice())
  }
}

// What an effect still out when its op's row was written records: the op
// ended, failing or timing out, before its output came back.
fn unfinished(name: &str) -> ServiceError {
  ServiceError::new(503, "UNFINISHED", format!("The effect {name} had not answered when its op ended"))
}

// What a replay asking for an effect its first run never made gets.
fn beyond(name: &str) -> ServiceError {
  unreachable(&format!("The effect {name}, which this op's first run never made,"))
}

// An effect this device cannot carry out, the same wherever it is asked for.
pub fn unreachable(what: &str) -> ServiceError {
  ServiceError::new(503, "UNREACHABLE", format!("{what} cannot be reached from this device"))
}

// How an effect is carried out: given the service call that asks for it, its
// input, and where to answer.
pub type Handler = Rc<dyn Fn(Rc<ServiceCall>, Value, Reply)>;

// The effects this device can carry out, by name.
#[derive(Default)]
pub struct Effects {
  handlers: BTreeMap<String, Handler>,
}

impl Effects {
  pub fn with(mut self, name: &str, handler: Handler) -> Effects {
    self.handlers.insert(name.to_string(), handler);
    self
  }

  // Carry out an effect, or answer that it cannot be reached from here.
  pub(crate) fn make(&self, call: Rc<ServiceCall>, name: &str, input: Value, reply: Reply) {
    match self.handlers.get(name) {
      Some(handler) => handler(call, input, reply),
      None => reply(Err(unreachable(&format!("The effect {name}")))),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::error::conflict;

  fn made(name: &str, output: Result<Value>) -> Made {
    Made { name: name.into(), input: json!({"n": 1}), output }
  }

  #[test]
  fn an_effect_is_kept_with_its_output_or_its_failure() {
    let cases = [made("call", Ok(json!(["red"]))), made("call", Err(conflict("taken")))];
    for made in cases {
      let kept = serde_json::to_value(&made).unwrap();
      assert_eq!(kept.get("output").is_some(), made.output.is_ok());
      assert_eq!(serde_json::from_value::<Made>(kept).unwrap(), made);
    }
  }

  #[test]
  fn a_first_run_records_its_effects_in_the_order_it_asked_for_them() {
    let mut first = EffectLog::of(None);
    let slots: Vec<usize> = ["call", "create", "fetch"]
      .iter()
      .map(|name| match first.next(name, &json!({"n": 1})) {
        Turn::Make(slot) => slot,
        Turn::Recorded(_) => panic!("a first run makes its effects"),
      })
      .collect();
    first.keep(slots[1], json!({"n": 2}), Ok(json!("second")));
    first.keep(slots[0], json!({"n": 1}), Ok(json!("first")));
    let outputs: Vec<std::result::Result<Value, String>> = first.made().unwrap().iter().map(|made| made.output.clone().map_err(|error| error.code)).collect();
    assert_eq!(outputs, [Ok(json!("first")), Ok(json!("second")), Err("UNFINISHED".to_string())], "in asked order, whatever order they came back in");
  }

  #[test]
  fn a_replay_hands_back_the_recorded_outputs_in_order() {
    let mut replay = EffectLog::of(Some(vec![made("call", Ok(json!(1))), made("create", Err(conflict("no")))]));
    let outputs: Vec<std::result::Result<Value, u16>> = ["fetch", "call", "call"]
      .iter()
      .map(|name| match replay.next(name, &Value::Null) {
        Turn::Recorded(output) => output.map_err(|error| error.status),
        Turn::Make(_) => panic!("a replay makes nothing"),
      })
      .collect();
    assert_eq!(outputs, [Ok(json!(1)), Err(409), Err(503)], "by order, whatever the name, and unreachable past the end");
    assert_eq!(replay.made(), None, "a replay writes nothing onto the row");
  }
}
