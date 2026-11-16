//! The core as an app reaches it through its runtime: a call as an identity
//! on this device, by its service key; and a ref found at once, read straight
//! from `[channel]`'s table. Watches are in `watch`.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::call_window::CallWindow;
use crate::carbon::Carbon;
use crate::error::{Result, bad_request, not_found};
use crate::services::carbon as local;
use crate::space;
use crate::store::Reader;
use crate::{Actor, Call, Db, Routing};

// How a call from the app answers it, once.
pub type Answer = Box<dyn FnOnce(Result<Value>)>;

impl Carbon {
  // A call from the app, by its service key, on a channel's handle or, with
  // none, the system handle; made as the identity named, or as no one. On a
  // channel, `[name]` reaches the service the channel names, and `[@carbon]`
  // runs in the system about that channel. A write answers once the watches
  // its commit owes a run have run; a GET at once.
  pub fn call(self: &Rc<Self>, space: Option<&str>, identity: Option<&str>, key: &str, input: Value, answer: Answer) {
    let made = self.actor(identity).and_then(|actor| Ok((actor, Call::from_key(key)?)));
    let (actor, call) = match made {
      Ok(made) => made,
      Err(error) => return answer(Err(error)),
    };
    let reached = match space {
      Some(id) if call.confined => about(id, input).map(|input| (Rc::clone(self.system()), input)),
      Some(id) => self.service_space(id, &call).map(|space| (space, input)),
      None => Ok((Rc::clone(self.system()), input)),
    };
    let (space, input) = match reached {
      Ok(reached) => reached,
      Err(error) => return answer(Err(error)),
    };
    let settling = Rc::clone(self);
    self.submit(&space, actor, call, input, Box::new(move |settled| {
      let owed = settled.as_ref().map_or_else(|_| vec![], |settled| settled.watches.clone());
      settling.when_run(owed, Box::new(move || answer(settled.map(|settled| settled.value))))
    }));
  }

}

// A `[@carbon]` call made on a channel's handle is about that channel: its
// input names it, beside the service it may name.
fn about(channel: &str, input: Value) -> Result<Value> {
  let mut input = match input {
    Value::Null => serde_json::Map::new(),
    Value::Object(input) => input,
    _ => return Err(bad_request("A [@carbon] call on a channel takes an object, or nothing")),
  };
  input.insert("channel".into(), json!(channel));
  Ok(Value::Object(input))
}

impl Carbon {
  // Resume a service a system failure stopped, once the failure is fixed;
  // answers once the watches on it have run again.
  pub fn resume_space(self: &Rc<Self>, space: &str, answer: Answer) {
    let carbon = Rc::clone(self);
    Rc::clone(self).with_space(space, Box::new(move |opened| match opened {
      Ok(space) => {
        let rerun = carbon.resume(&space);
        carbon.when_run(rerun, Box::new(move || answer(Ok(Value::Null))));
      }
      Err(error) => answer(Err(error)),
    }));
  }

  // Who acts for a call from the app: an identity on this device, from the
  // device it holds here; or no one.
  pub fn actor(&self, identity: Option<&str>) -> Result<Actor> {
    let Some(id) = identity else { return Ok(Actor::default()) };
    let acting = local::acting(&self.system().store().reader()?, id)?;
    Ok(Actor { identity: Some(acting.id), device: Some(acting.device) })
  }

  // The ref in a channel to `kind:slug`: the channel it points to, its kind, name and slug.
  pub fn find_ref(&self, channel: &str, kind: &str, slug: &str) -> Result<Value> {
    let found = self.read_tables(channel, |db| db.get("SELECT channel, kind, name, slug FROM ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)]))?;
    match found.is_null() {
      true => Err(not_found(format!("No ref to {kind}:{slug} here"))),
      false => Ok(found),
    }
  }

  // Read a space's service tables directly, read-only and over committed
  // state, at once: the one way the core reads a service's records without
  // calling it, for an answer an app needs synchronously.
  pub fn read_tables<T>(&self, space: &str, work: impl FnOnce(&dyn Db) -> Result<T>) -> Result<T> {
    let path = self.space_path(space);
    if !path.exists() {
      return Err(space::missing(space));
    }
    let window = CallWindow { service: true, op: None };
    let reader = Reader::open(&path)?;
    let read = work(&reader.scoped(window));
    reader.end()?;
    read
  }

}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{alice, carbon, now};

  #[test]
  fn a_call_acts_as_an_identity_on_this_device_from_the_device_it_holds_here() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    assert_eq!(carbon.actor(Some(&identity.id)).unwrap(), Actor { identity: Some(identity.id.clone()), device: Some(identity.device.clone()) });
    assert_eq!(carbon.actor(None).unwrap(), Actor::default());
    assert_eq!(carbon.actor(Some("elsewhere")).unwrap_err().status, 404);
    let made = now(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), "POST [notes]/run", json!({"do": []}), answer));
    assert!(made.is_ok());
    let refused = [(Some("elsewhere"), "GET [channel]/refs", 404), (Some(identity.id.as_str()), "not a key", 400)];
    for (acting, key, status) in refused {
      assert_eq!(now(|answer| carbon.call(Some(&identity.personal), acting, key, Value::Null, answer)).unwrap_err().status, status, "{key}");
    }
  }

  #[test]
  fn finds_a_ref_at_once_in_the_channel_table_and_refuses_one_that_is_not_there() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let found = carbon.find_ref(&identity.channel, "personal", "alice").unwrap();
    assert_eq!(found["channel"], json!(identity.personal));
    assert_eq!(carbon.find_ref(&identity.channel, "channel", "lobby").unwrap_err().status, 404);
    assert_eq!(carbon.find_ref("nowhere", "channel", "lobby").unwrap_err().status, 404);
  }
}
