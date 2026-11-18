//! The first milestone's default assignment policy: every device an
//! identity's `[identity]` lists is a member, at `manage`, of every service
//! space its node syncs, which is to say every service of its own and personal
//! channels. The device whose `[@carbon]` op calls for a change makes it,
//! once: enrolling a device adds it to every space the identity's node
//! follows (`crate::nodes::pairing`), and a space created here gets every
//! listed device as it is followed. Adding the listed devices whenever this
//! device follows a space also repairs what a crash cut short; adding a member
//! already added changes nothing.

use std::rc::Rc;
use std::sync::Arc;

use p2panda::spaces::Space;
use p2panda::{AccessLevel, VerifyingKey};

use super::on_network;
use crate::carbon::Carbon;
use crate::error::{Result, internal};
use crate::ops::oplog::LogRow;
use crate::services::carbon as local;

impl Carbon {
  // Add the devices the space's identity lists, but this one, that are not yet its members.
  pub(crate) fn add_listed_devices(self: &Rc<Self>, space: &str) {
    let Some(followed) = self.followed(space) else { return };
    let devices = match self.listed_devices(&followed.identity) {
      Ok(devices) => devices,
      Err(_) => return,
    };
    let missing: Vec<String> = devices.into_iter().filter(|device| *device != followed.device && !followed.adding.borrow().contains(device)).collect();
    if missing.is_empty() {
      return;
    }
    followed.adding.borrow_mut().extend(missing.iter().cloned());
    let target = Arc::clone(&followed.space);
    let adding = missing.clone();
    let added = async move {
      let keys = adding.iter().map(|device| device.parse::<VerifyingKey>().map_err(|error| internal(format!("{device} is not a device key: {error}")))).collect::<Result<Vec<_>>>()?;
      add_members(&target, &keys).await
    };
    let (carbon, space) = (Rc::clone(self), space.to_string());
    on_network(self, added, move |added: Result<()>| {
      if let Some(followed) = carbon.followed(&space) {
        missing.iter().for_each(|device| {
          followed.adding.borrow_mut().remove(device);
        });
      }
      if let Err(error) = added {
        eprintln!("[carbondb] {space}: {}", error.message);
      }
    });
  }

  // The devices an identity on this device lists in its `[identity]`.
  fn listed_devices(self: &Rc<Self>, identity: &str) -> Result<Vec<String>> {
    let held = local::acting(&self.system.store().reader()?, identity)?;
    let devices = self.named(&held.channel, "identity")?;
    let rows = self.read_tables(&devices, |db| db.all("SELECT pubkey FROM device ORDER BY pubkey", &[]))?;
    Ok(rows.iter().filter_map(|row| row["pubkey"].as_str().map(str::to_string)).collect())
  }

  // The p2panda spaces an identity's node follows here.
  pub(crate) fn spaces_of(&self, identity: &str) -> Vec<Arc<Space<LogRow>>> {
    self.nodes.spaces.followed_ids().iter().filter_map(|space| self.followed(space)).filter(|followed| followed.identity == identity).map(|followed| Arc::clone(&followed.space)).collect()
  }
}

// Add devices to a space at `manage`, those that are not yet its members.
pub(crate) async fn add_members(space: &Space<LogRow>, devices: &[VerifyingKey]) -> Result<()> {
  let members: Vec<String> = space.members().await.map_err(|error| internal(format!("Cannot read a space's members: {error}")))?.into_iter().map(|(member, _)| member.to_hex()).collect();
  for device in devices.iter().filter(|device| !members.contains(&device.to_hex())) {
    space.add(*device, AccessLevel::Manage).await.map_err(|error| internal(format!("Cannot add device {device}: {error}")))?;
  }
  Ok(())
}
