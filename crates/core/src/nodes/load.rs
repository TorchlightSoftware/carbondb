//! Loading channels in the `ref` sync mode: a node holds each identity's own
//! channel, and every channel the channels it holds point to, with every
//! service they name. It watches `GET [channel]/refs` and `GET
//! [channel]/services` on each channel it holds; a watch answers as soon as
//! it is registered, so the same watches load everything at start and
//! whatever appears later, as the ops naming it apply. A channel this device
//! does not hold yet is created here and its p2panda space joined, which
//! completes once a device of the identity adds this one (`members`); a
//! service's store is opened, running the package registered under its name,
//! and takes up what the channel's log addresses to it.

use std::rc::Rc;

use serde_json::Value;

use crate::carbon::Carbon;
use crate::channels::channel::Channel;
use crate::error::Result;

impl Carbon {
  // Load an identity's own channel, and through it every channel and service it reaches.
  pub(crate) fn load_identity(self: &Rc<Self>, identity: &str, channel: &str) {
    self.load_channel(channel, identity);
  }

  // Hold a channel, follow its p2panda space, and watch what it names.
  fn load_channel(self: &Rc<Self>, channel: &str, identity: &str) {
    if !self.nodes.loading.borrow_mut().insert(channel.to_string()) {
      return;
    }
    let held = match self.held_or_created(channel, identity) {
      Ok(held) => held,
      Err(error) => return eprintln!("[carbondb] channel {channel} is not loaded: {}", error.message),
    };
    self.join_p2panda_space(channel, identity);
    let (services, refs) = (Rc::downgrade(self), Rc::downgrade(self));
    let on_services = Rc::new(move |listed: Result<Value>| {
      let (Some(carbon), Ok(listed)) = (services.upgrade(), listed) else { return };
      for name in listed.as_array().into_iter().flatten().filter_map(|service| service["name"].as_str()) {
        if let Err(error) = carbon.named_store(&held, name) {
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

  // A channel this device holds, created here if it does not hold it yet.
  fn held_or_created(self: &Rc<Self>, channel: &str, identity: &str) -> Result<Rc<Channel>> {
    match self.channel(channel) {
      Ok(held) => Ok(held),
      Err(_) => self.create_channel(channel, identity),
    }
  }
}
