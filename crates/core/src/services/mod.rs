//! The services this device runs: `[@carbon]`, the one the core implements
//! itself; the builtin packages `[space]`, `[identity]` and `[device]`; and the
//! app services the runtime registers at startup. A service is its name,
//! whether it is local (`[@name]`, run once on this device) or replicated, its
//! version, its `sql` binding, the `init` that sets up its schema, and its entry
//! point. CarbonDB routes a call to an entry point by service name only; the
//! entry point routes method and resource itself.

pub mod carbon;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::json;

use crate::error::{Result, bad_request, conflict, not_found};
use crate::namespace;
use crate::service_call::ServiceCall;
use crate::{Db, HandlerContext, Package};

// The packages CarbonDB ships, linked into the library.
const BUILTINS: [Package; 3] = [carbondb_space::PACKAGE, carbondb_identity::PACKAGE, carbondb_device::PACKAGE];

// What sets up a service's schema, given its `db` binding.
pub type Init = Rc<dyn Fn(Rc<dyn Db>) -> Result<()>>;

// An entry point written against ServiceBindings: a package linked into the
// library, or one hosted by a runtime, such as npm.
pub type Entry = Rc<dyn Fn(Rc<dyn HandlerContext>)>;

pub struct Service {
  pub name: String,
  pub local: bool,
  // The hash of the package that handles its calls, which every op on it carries.
  pub version: String,
  pub binding: Option<String>,
  pub init: Init,
  // Every service but `[@carbon]` sees its service call only as a handler context.
  pub(crate) entry: Rc<dyn Fn(Rc<ServiceCall>)>,
}

impl Service {
  pub fn replicated(name: &str, version: &str, binding: Option<&str>, init: Init, entry: Entry) -> Service {
    Service {
      name: name.into(),
      local: false,
      version: version.into(),
      binding: binding.map(str::to_string),
      init,
      entry: Rc::new(move |call: Rc<ServiceCall>| entry(call)),
    }
  }

  fn package(package: &Package) -> Service {
    let (init, entry) = (package.init, package.entry);
    Service::replicated(package.name, package.version, package.binding, Rc::new(move |db: Rc<dyn Db>| init(db.as_ref())), Rc::new(entry))
  }

  // The namespace its statements run in: `chat/db/`. None for `[@carbon]`, the
  // core's own, whose statements run unscoped.
  pub fn prefix(&self) -> Result<Option<String>> {
    match (&self.binding, self.local) {
      (Some(binding), _) => Ok(Some(namespace::prefix(&self.name, binding))),
      (None, true) => Ok(None),
      (None, false) => Err(bad_request(format!("Service '{}' declares no sql binding, so it has no tables", self.name))),
    }
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
    for service in std::iter::once(carbon::service()).chain(BUILTINS.iter().map(Service::package)) {
      services.register(service).expect("builtin names are distinct");
    }
    services
  }

  pub fn register(&self, service: Service) -> Result<Rc<Service>> {
    namespace::check_parts(std::iter::once(service.name.as_str()).chain(service.binding.as_deref()))?;
    let mut registered = self.registered.borrow_mut();
    if registered.contains_key(&service.name) {
      return Err(conflict(format!("A service '{}' is already registered on this device", service.name)));
    }
    let service = Rc::new(service);
    registered.insert(service.name.clone(), Rc::clone(&service));
    Ok(service)
  }

  pub fn find(&self, name: &str) -> Option<Rc<Service>> {
    self.registered.borrow().get(name).cloned()
  }

  // A service a space names, which this device must run.
  pub fn get(&self, name: &str) -> Result<Rc<Service>> {
    self.find(name).ok_or_else(|| missing(name))
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

  // The version an op on a service carries: the registered service's, or
  // SERVICE_MISSING when this device does not run it.
  pub fn version(&self, name: &str) -> String {
    self.find(name).map_or_else(|| SERVICE_MISSING.to_string(), |service| service.version.clone())
  }
}

// The version of an op made on this device for a service it does not run. No
// handler sees it: the op is refused when its turn comes, which also keeps it
// from running again and from being published.
pub const SERVICE_MISSING: &str = "service missing";

// Where a service a space names leads when this device does not run it.
pub fn missing(name: &str) -> crate::error::ServiceError {
  not_found(format!("Service '{name}' is not registered on this device")).with_details(json!({"service": name}))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn service(name: &str) -> Service {
    Service::replicated(name, "v1", Some("db"), Rc::new(|_: Rc<dyn Db>| Ok(())), Rc::new(|_| {}))
  }

  #[test]
  fn starts_with_the_builtins_and_registers_each_name_once() {
    let services = Services::builtin();
    let builtins: Vec<(String, bool, Option<String>)> =
      ["carbon", "space", "identity", "device"].iter().map(|name| services.get(name).unwrap()).map(|service| (service.name.clone(), service.local, service.binding.clone())).collect();
    let db = || Some("db".to_string());
    assert_eq!(builtins, [("carbon".into(), true, None), ("space".into(), false, db()), ("identity".into(), false, db()), ("device".into(), false, db())]);
    services.register(service("chat")).unwrap();
    for name in ["chat", "space"] {
      assert_eq!(services.register(service(name)).err().unwrap().status, 409, "{name}");
    }
    assert_eq!(services.version("chat"), "v1");
    assert_eq!(services.get("chat").unwrap().prefix().unwrap(), Some("chat/db/".into()));
  }

  #[test]
  fn refuses_a_name_that_would_nest_inside_another_service_namespace() {
    let services = Services::builtin();
    services.register(service("chat")).unwrap();
    let nested = [Service::replicated("chat/db", "v1", Some("db"), Rc::new(|_: Rc<dyn Db>| Ok(())), Rc::new(|_| {})), Service::replicated("notes", "v1", Some("db/x"), Rc::new(|_: Rc<dyn Db>| Ok(())), Rc::new(|_| {}))];
    for service in nested {
      let name = service.name.clone();
      assert_eq!(services.register(service).err().unwrap().status, 400, "{name}");
      assert!(services.find(&name).is_none(), "{name}");
    }
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
