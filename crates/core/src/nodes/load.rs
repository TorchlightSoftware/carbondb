//! Loading spaces in the `ref` sync mode: a node holds each identity's own
//! channel, and every channel and service the channels it holds name. It
//! watches `GET [channel]/refs` and `GET [channel]/services` on each channel
//! it holds; a watch answers as soon as it is registered, so the same watches
//! load everything at start and whatever appears later, as the ops naming it
//! apply. A space this device does not hold yet is created here, running the
//! package registered under its name, and its p2panda space joined, which
//! completes once a device of the identity adds this one (`members`).

use std::rc::Rc;

use serde_json::Value;

use crate::carbon::Carbon;
use crate::channels::service::Runs;
use crate::error::Result;

impl Carbon {
  // Load an identity's own channel, and through it every channel and service it reaches.
  pub(crate) fn load_identity(self: &Rc<Self>, identity: &str, channel: &str) {
    self.load_channel(channel, identity);
  }

  // Hold a channel, and watch what it names.
  fn load_channel(self: &Rc<Self>, channel: &str, identity: &str) {
    if !self.nodes.loading.borrow_mut().insert(channel.to_string()) {
      return;
    }
    if let Err(error) = self.load(channel, Runs { package: "channel".into(), name: "channel".into(), channel: Some(channel.to_string()) }, identity) {
      return eprintln!("[carbondb] channel {channel} is not loaded: {}", error.message);
    }
    let (services, refs) = (Rc::downgrade(self), Rc::downgrade(self));
    let (named_in, identity_of) = (channel.to_string(), identity.to_string());
    let on_services = Rc::new(move |listed: Result<Value>| {
      let (Some(carbon), Ok(listed)) = (services.upgrade(), listed) else { return };
      for service in listed.as_array().into_iter().flatten() {
        let (Some(space), Some(name), Some(package)) = (service["service"].as_str(), service["name"].as_str(), service["package"].as_str()) else { continue };
        let runs = Runs { package: package.to_string(), name: name.to_string(), channel: Some(named_in.clone()) };
        if let Err(error) = carbon.load(space, runs, &identity_of) {
          eprintln!("[carbondb] [{name}] is not loaded: {}", error.message);
        }
      }
    });
    let identity_of = identity.to_string();
    let on_refs = Rc::new(move |listed: Result<Value>| {
      let (Some(carbon), Ok(listed)) = (refs.upgrade(), listed) else { return };
      listed.as_array().into_iter().flatten().filter_map(|reference| reference["channel"].as_str()).for_each(|referred| carbon.load_channel(referred, &identity_of));
    });
    let watched = self.watch(channel, Some(identity), "GET [channel]/services", on_services).and_then(|_| self.watch(channel, Some(identity), "GET [channel]/refs", on_refs));
    if let Err(error) = watched {
      eprintln!("[carbondb] channel {channel} is not watched: {}", error.message);
    }
  }

  // Hold a space this device does not hold yet, synced by the identity's node, and follow its p2panda space.
  fn load(self: &Rc<Self>, space: &str, runs: Runs, identity: &str) -> Result<()> {
    if self.held(space)?.is_none() {
      self.create_space(space, runs)?;
      self.synced_by(space, identity)?;
    }
    self.join_p2panda_space(space, identity);
    Ok(())
  }
}
