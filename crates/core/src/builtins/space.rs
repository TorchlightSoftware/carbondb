//! `[space]`: a space's origin, its refs to other spaces, and later its members
//! and manage list. Every POST but the origin needs manage on the space.

use serde_json::{Value, json};

use crate::call::{Actor, Call, Outcome};
use crate::error::{Result, not_built, single_node};
use crate::space::{self, Origin, Ref};
use crate::store::{Side, Store};

pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/builtins/space/001.sql")];

pub fn handle(store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  match (call.method(), call.resource.as_str()) {
    ("POST", "/origin") => Outcome::write(store, |store| {
      let origin: Origin = serde_json::from_value(input)?;
      space::write_origin(store, &origin)?;
      Ok(json!(origin))
    }),
    ("POST", "/ref") => Outcome::write(store, |store| {
      space::require_manage(store, actor.identity.as_deref())?;
      let reference: Ref = serde_json::from_value(input)?;
      space::add_ref(store, &reference)?;
      Ok(json!(reference))
    }),
    ("GET", "/refs") => Outcome::read(store, |store| Ok(json!(space::refs(store, Side::Reader)?))),
    ("POST", "/members") => Err(single_node("Adding a member")),
    ("POST", "/manage") => Err(not_built("Changing a manage list")),
    _ => Err(call.unknown()),
  }
}
