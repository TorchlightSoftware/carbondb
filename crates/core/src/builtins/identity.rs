//! `[identity]`: the identity whose identity space this is. Its origin is
//! written when the identity is set up, so the only call reads it.

use serde_json::Value;

use crate::call::{Actor, Call, Outcome};
use crate::error::Result;
use crate::identity;
use crate::store::{Side, Store};

pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/builtins/identity/001.sql")];

pub fn handle(store: &Store, _actor: &Actor, call: &Call, _input: Value) -> Result<Outcome> {
  match (call.method(), call.resource.as_str()) {
    ("GET", "/origin") => Outcome::read(store, |store| identity::origin(store, Side::Reader)),
    _ => Err(call.unknown()),
  }
}
