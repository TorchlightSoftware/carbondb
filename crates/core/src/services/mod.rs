//! The services this device runs: `[@carbon]`, the one the core implements
//! itself; the builtin packages `[space]`, `[identity]` and `[device]`; and the
//! app services the runtime registers at startup. A service is its name,
//! whether it is local (`[@name]`, run once on this device) or replicated, its
//! migrations, and its entry point. CarbonDB routes a call to an entry point by
//! service name only; the entry point routes method and resource itself.

pub mod carbon;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::json;

use crate::error::{Result, bad_request, conflict, not_found};
use crate::frame::Frame;
use crate::{Entry, Package};

// The packages CarbonDB ships, linked into the library.
const BUILTINS: [Package; 3] = [carbondb_space::PACKAGE, carbondb_identity::PACKAGE, carbondb_device::PACKAGE];

pub struct Service {
  pub name: String,
  pub local: bool,
  pub migrations: Vec<String>,
  // Every service but `[@carbon]` sees its frame only as an `Invocation`.
  pub(crate) entry: Rc<dyn Fn(Rc<Frame>)>,
}

impl Service {
  // A replicated service whose entry point is written against the service API:
  // a package linked into the library, or one hosted by a runtime, such as npm.
  pub fn replicated(name: &str, migrations: Vec<String>, entry: Entry) -> Service {
    Service { name: name.into(), local: false, migrations, entry: Rc::new(move |frame: Rc<Frame>| entry(frame)) }
  }

  fn package(package: &Package) -> Service {
    let entry = package.entry;
    Service::replicated(package.name, package.migrations.iter().map(|sql| sql.to_string()).collect(), Rc::new(entry))
  }
}

#[derive(Default)]
pub struct Services {
  registered: RefCell<BTreeMap<String, Rc<Service>>>,
}

impl Services {
  // The services every CarbonDB starts with.
  pub fn builtin() -> Services {
    let services = Services::default();
    let carbon = Service { name: "carbon".into(), local: true, migrations: vec![], entry: Rc::new(carbon::entry) };
    for service in std::iter::once(carbon).chain(BUILTINS.iter().map(Service::package)) {
      services.register(service).expect("builtin names are distinct");
    }
    services
  }

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
    Service::replicated(name, vec!["CREATE TABLE t (id TEXT PRIMARY KEY)".into()], Rc::new(|_| {}))
  }

  #[test]
  fn starts_with_the_builtins_and_registers_each_name_once() {
    let services = Services::builtin();
    let builtins: Vec<(String, bool, usize)> = ["carbon", "space", "identity", "device"].iter().map(|name| services.get(name).unwrap()).map(|service| (service.name.clone(), service.local, service.migrations.len())).collect();
    assert_eq!(builtins, [("carbon".into(), true, 0), ("space".into(), false, 1), ("identity".into(), false, 1), ("device".into(), false, 1)]);
    services.register(service("chat")).unwrap();
    for name in ["chat", "space"] {
      assert_eq!(services.register(service(name)).unwrap_err().status, 409, "{name}");
    }
    assert_eq!(services.version("chat"), 1);
  }

  #[test]
  fn names_what_is_missing() {
    let services = Services::builtin();
    assert!(services.require_registered(&["space".into()]).is_ok());
    let error = services.require_registered(&["space".into(), "crm".into()]).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"services": ["crm"]}))));
    assert_eq!(services.get("crm").err().unwrap().details, Some(json!({"service": "crm"})));
  }
}
