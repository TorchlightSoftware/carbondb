//! `POST [@carbon]/space`: a space nested in the calling one. The calling space
//! must accept the ref first, so a caller without manage there creates nothing.
//! The new space's id is this call's op hash, so a retry after a crash reaches
//! the same space.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::json;

use super::steps::{self, Step, Target};
use crate::error::{Result, bad_request};
use crate::frame::Frame;
use crate::Invocation;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceInput {
  #[serde(default = "default_kind")]
  kind: String,
  name: String,
  #[serde(default)]
  services: Vec<String>,
  manage: Option<Vec<String>>,
  #[serde(default)]
  cascade: Vec<String>,
  slug: Option<String>,
}

fn default_kind() -> String {
  "space".to_string()
}

// Settle with the ref the calling space accepted.
pub fn create(frame: Rc<Frame>) {
  let planned = plan(&frame);
  let caller = Rc::clone(&frame);
  match planned {
    Ok(planned) => steps::run(frame, planned, Box::new(move |result| {
      let _ = caller.settle(result.map(|mut returned| returned.swap_remove(0)));
    })),
    Err(error) => {
      let _ = caller.settle(Err(error));
    }
  }
}

fn plan(frame: &Frame) -> Result<VecDeque<Step>> {
  let actor = frame.actor.clone();
  let author = actor.identity.clone().ok_or_else(|| bad_request("Creating a space needs an acting identity"))?;
  let request: SpaceInput = serde_json::from_value(frame.input.clone())?;
  frame.carbon().services().require_registered(&request.services)?;
  let id = frame.op().map(|op| op.hash.clone()).expect("a POST is an op");
  let origin = json!({
    "kind": request.kind,
    "name": request.name,
    "services": request.services,
    "manage": request.manage.unwrap_or_else(|| vec![author]),
  });
  let reference = json!({
    "space": id,
    "kind": request.kind,
    "name": request.name,
    "slug": request.slug,
    "cascade": request.cascade,
  });
  Ok(VecDeque::from([
    Step::Call { target: Target::Here, actor: actor.clone(), key: "POST [space]/ref", input: reference },
    Step::Create(id.clone()),
    Step::Call { target: Target::Space(id.clone()), actor, key: "POST [space]/origin", input: origin },
    Step::Provision(id, request.services),
  ]))
}
