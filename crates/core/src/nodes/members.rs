//! The first milestone's default assignment policy: every device an
//! identity's `[identity]` lists is a member, at `manage`, of every service
//! space its node syncs, which is to say every service of its own and personal
//! channels, those added after a device enrolled included. The work is derived
//! from state, not from the op that called for it, so a crash loses nothing:
//! it runs whenever this device follows a space, and whenever `[identity]`
//! changes, and adding a member already added changes nothing.

use std::rc::Rc;
use std::sync::Arc;

use p2panda::AccessLevel;
use p2panda::VerifyingKey;

use super::on_network;
use crate::carbon::Carbon;
use crate::error::{Result, internal};
use crate::services::carbon as local;

impl Carbon {
  // Add the devices an identity lists to every space its node syncs here.
  pub(crate) fn reconcile_members(self: &Rc<Self>, identity: &str) {
    let spaces: Vec<String> = self.nodes.spaces.followed_ids().into_iter().filter(|space| self.followed(space).is_some_and(|followed| followed.identity == identity)).collect();
    spaces.iter().for_each(|space| self.add_listed_devices(space));
  }

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
      let members: Vec<String> = target.members().await.map_err(|error| internal(format!("Cannot read a space's members: {error}")))?.into_iter().map(|(member, _)| member.to_hex()).collect();
      for device in adding.iter().filter(|device| !members.contains(device)) {
        let key = device.parse::<VerifyingKey>().map_err(|error| internal(format!("{device} is not a device key: {error}")))?;
        target.add(key, AccessLevel::Manage).await.map_err(|error| internal(format!("Cannot add device {device}: {error}")))?;
      }
      Ok(())
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

  // A commit to a space: when it is an identity's `[identity]`, the devices it lists join its spaces.
  pub(crate) fn listed_changed(self: &Rc<Self>, space: &str) {
    let Some(followed) = self.followed(space) else { return };
    if self.opened(space).is_some_and(|opened| opened.runs.package == "identity") {
      self.reconcile_members(&followed.identity);
    }
  }
}
