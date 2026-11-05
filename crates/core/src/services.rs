//! The services this device runs: CarbonDB's builtins, and the app services
//! the runtime registers at startup. A service is its name, whether it is local
//! (`[@name]`, run once on this device) or replicated, its migrations, and its
//! entry point. CarbonDB routes a call to an entry point by service name only;
//! the entry point routes method and resource itself.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::json;

use crate::error::{Result, bad_request, conflict, not_found};
use crate::invocation::Invocation;

// An entry point: it receives each call to its service as an invocation, and
// settles it once, now or later. Rust builtins, npm services and WASM packages
// all take this shape.
pub type Entry = Rc<dyn Fn(Rc<Invocation>)>;

pub struct Service {
  pub name: String,
  pub local: bool,
  pub migrations: Vec<String>,
  pub entry: Entry,
}

#[derive(Default)]
pub struct Services {
  registered: RefCell<BTreeMap<String, Rc<Service>>>,
}

impl Services {
  pub fn register(&self, service: Service) -> Result<()> {
    let mut registered = self.registered.borrow_mut();
    if registered.contains_key(&service.name) {
      return Err(conflict(format!("A service '{}' is already registered on this device", service.name)));
    }
    registered.insert(service.name.clone(), Rc::new(service));
    Ok(())
  }

  pub fn find(&self, name: &str) -> Option<Rc<Service>> {
    self.registered.borrow().get(name).cloned()
  }

  // A service a space names, which this device must run.
  pub fn get(&self, name: &str) -> Result<Rc<Service>> {
    self.find(name).ok_or_else(|| not_found(format!("Service '{name}' is not registered on this device")).with_details(json!({"service": name})))
  }

  // Refuse a new space naming a service this device cannot run.
  pub fn require_registered(&self, names: &[String]) -> Result<()> {
    let missing: Vec<&String> = names.iter().filter(|name| self.find(name).is_none()).collect();
    if missing.is_empty() {
      return Ok(());
    }
    let list = missing.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", ");
    Err(bad_request(format!("Services not registered on this device: {list}")).with_details(json!({"services": missing})))
  }

  // The schema version an op on a service carries: how many migrations it declares.
  pub fn version(&self, name: &str) -> usize {
    self.find(name).map_or(0, |service| service.migrations.len())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn service(name: &str) -> Service {
    Service { name: name.into(), local: false, migrations: vec!["CREATE TABLE t (id TEXT PRIMARY KEY)".into()], entry: Rc::new(|_| {}) }
  }

  #[test]
  fn registers_each_name_once() {
    let services = Services::default();
    services.register(service("chat")).unwrap();
    assert_eq!(services.register(service("chat")).unwrap_err().status, 409);
    assert_eq!(services.version("chat"), 1);
  }

  #[test]
  fn names_what_is_missing() {
    let services = Services::default();
    services.register(service("chat")).unwrap();
    assert!(services.require_registered(&["chat".into()]).is_ok());
    let error = services.require_registered(&["chat".into(), "crm".into()]).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"services": ["crm"]}))));
    assert_eq!(services.get("crm").err().unwrap().details, Some(json!({"service": "crm"})));
  }
}
