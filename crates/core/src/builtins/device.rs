//! `[device]`: in an identity space, the device keys it authorizes; in a
//! personal space, each device's name and tags. Writing a ref needs manage.

use serde_json::{Value, json};

use crate::call::{Actor, Call, Outcome};
use crate::error::{Result, single_node};
use crate::identity::{self, DeviceRef};
use crate::space;
use crate::store::{Side, Store};

pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/builtins/device/001.sql")];

pub fn handle(store: &Store, actor: &Actor, call: &Call, input: Value) -> Result<Outcome> {
  match (call.method(), call.resource.as_str()) {
    ("POST", "/origin") => Err(single_node("Adding a device")),
    ("GET", "/devices") => Outcome::read(store, |store| Ok(json!(identity::devices(store, Side::Reader)?))),
    ("POST", "/ref") => Outcome::write(store, |store| {
      space::require_manage(store, actor.identity.as_deref())?;
      let device: DeviceRef = serde_json::from_value(input)?;
      identity::put_device_ref(store, &device)?;
      Ok(json!(device))
    }),
    ("GET", "/refs") => Outcome::read(store, |store| Ok(json!(identity::device_refs(store, Side::Reader)?))),
    _ => Err(call.unknown()),
  }
}
