//! The core as an app reaches it through its runtime: a call as an identity
//! on this device, by its service key; and a ref found at once, read straight
//! from `[space]`'s table. Watches are in `watch`.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::call_window::CallWindow;
use crate::carbon::{Carbon, Opened};
use crate::error::{Result, not_found};
use crate::services::carbon as local;
use crate::space;
use crate::store::Reader;
use crate::{Actor, Call, Db, Routing};

// How a call from the app answers it, once.
pub type Answer = Box<dyn FnOnce(Result<Value>)>;

impl Carbon {
  // A call from the app, by its service key, to a space or, with none, to the
  // system handle; made as the identity named, or as no one. It answers once
  // the watches on its space have run against its commit.
  pub fn call(self: &Rc<Self>, space: Option<&str>, identity: Option<&str>, key: &str, input: Value, answer: Answer) {
    let made = self.actor(identity).and_then(|actor| Ok((actor, Call::from_key(key)?)));
    let (actor, call) = match made {
      Ok(made) => made,
      Err(error) => return answer(Err(error)),
    };
    let carbon = Rc::clone(self);
    self.reach(space, Box::new(move |space| match space {
      Ok(space) => {
        let (settling, id) = (Rc::clone(&carbon), space.id.clone());
        carbon.submit(&space, actor, call, input, Box::new(move |settled| settling.when_settled(&id, Box::new(move || answer(settled.map(|settled| settled.value))))))
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

  // The ref in a space to `kind:slug`: the space it points to, its kind, name and slug.
  pub fn find_ref(&self, space: &str, kind: &str, slug: &str) -> Result<Value> {
    let found = self.read_tables(space, "space", |db| db.get("SELECT space, kind, name, slug FROM ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)]))?;
    match found.is_null() {
      true => Err(not_found(format!("No ref to {kind}:{slug} here"))),
      false => Ok(found),
    }
  }

  // Read a service's tables in a space directly, namespaced, read-only and
  // over committed state, at once: the one way the core reads a service's
  // records without calling it, for an answer an app needs synchronously.
  pub fn read_tables<T>(&self, space: &str, service: &str, work: impl FnOnce(&dyn Db) -> Result<T>) -> Result<T> {
    let path = self.space_path(space);
    if !path.exists() {
      return Err(space::missing(space));
    }
    let window = CallWindow { prefix: self.services().get(service)?.prefix()?, op: None };
    let reader = Reader::open(&path)?;
    let read = work(&reader.scoped(window));
    reader.end()?;
    read
  }

  // A space by id, or the system handle's root space when none is named.
  fn reach(self: &Rc<Self>, space: Option<&str>, opened: Opened) {
    match space {
      Some(id) => Rc::clone(self).with_space(id, opened),
      None => opened(Ok(Rc::clone(self.system()))),
    }
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
    let refused = [(Some("elsewhere"), "GET [space]/refs", 404), (Some(identity.id.as_str()), "not a key", 400)];
    for (acting, key, status) in refused {
      assert_eq!(now(|answer| carbon.call(Some(&identity.personal), acting, key, Value::Null, answer)).unwrap_err().status, status, "{key}");
    }
  }

  #[test]
  fn finds_a_ref_at_once_in_the_space_table_and_refuses_one_that_is_not_there() {
    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let found = carbon.find_ref(&identity.space, "personal", "alice").unwrap();
    assert_eq!(found["space"], json!(identity.personal));
    assert_eq!(carbon.find_ref(&identity.space, "space", "lobby").unwrap_err().status, 404);
    assert_eq!(carbon.find_ref("nowhere", "space", "lobby").unwrap_err().status, 404);
  }
}
