//! The services this device runs: `[@carbon]`, the one the core implements
//! itself; the builtin packages `[channel]` and `[identity]`; and the app
//! services the runtime registers at startup. A service is its name, whether
//! it is local (`[@name]`, run once on this device) or replicated, its
//! version, its `sql` binding, the time every call to it must settle within,
//! the effects it makes again on every run, the `setup` that sets up its
//! schema, and its entry point. Each space runs one service; CarbonDB routes a
//! call to that service's entry point, which routes method and resource itself.
//!
//! Every service but `[@carbon]` is a package, a component the WASM host runs
//! (`crate::wasm`), registered under the name the app gives it, which
//! overrides the canonical name in its service.toml.

pub mod carbon;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use serde_json::json;

use crate::error::{Result, bad_request, conflict, not_found};
use crate::service_call::ServiceCall;
use crate::wasm::Package;
use crate::{Db, HandlerContext};

// The packages CarbonDB ships, embedded in the library as the components
// `scripts/build-builtins` built and packaged from `crates/services/`. Each
// is versioned by the hash of its bytes, so devices on different releases of
// CarbonDB agree whenever a builtin is unchanged.
pub const BUILTINS: [(&str, &[u8]); 2] = [("channel", include_bytes!("../../builtins/channel.wasm")), ("identity", include_bytes!("../../builtins/identity.wasm"))];

// How long a call to a service may take to settle when its service.toml names no timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

// What sets up a service's schema, given its `db` binding.
pub type Setup = Rc<dyn Fn(Rc<dyn Db>) -> Result<()>>;

// An entry point written against ServiceBindings, running here as host code,
// as the core's tests run theirs; a package's runs on the WASM host.
pub type Entry = Rc<dyn Fn(Rc<dyn HandlerContext>)>;

pub struct Service {
  pub name: String,
  pub local: bool,
  // The hash of the package that handles its calls, which every op on it carries.
  pub version: String,
  pub binding: Option<String>,
  // Every top-level call to it must settle within this, or it is settled with a timeout.
  pub timeout: Duration,
  // The effects it makes again on every run of an op, by name; every other is
  // made on an op's first run only.
  pub always: Vec<String>,
  pub setup: Setup,
  // Every service but `[@carbon]` sees its service call only as a handler context.
  pub(crate) entry: Rc<dyn Fn(Rc<ServiceCall>)>,
}

impl Service {
  pub fn replicated(name: &str, version: &str, binding: Option<&str>, timeout: Duration, setup: Setup, entry: Entry) -> Service {
    Service {
      name: name.into(),
      local: false,
      version: version.into(),
      binding: binding.map(str::to_string),
      timeout,
      always: vec![],
      setup,
      entry: Rc::new(move |call: Rc<ServiceCall>| entry(call)),
    }
  }
}

#[derive(Default)]
pub struct Services {
  registered: RefCell<BTreeMap<String, Rc<Service>>>,
}

impl Services {
  // The services every CarbonDB starts with.
  pub fn builtin() -> Result<Services> {
    let services = Services::default();
    services.register(carbon::service())?;
    for (name, bytes) in BUILTINS {
      services.register(Package::read(bytes)?.service(name))?;
    }
    Ok(services)
  }

  pub fn register(&self, service: Service) -> Result<Rc<Service>> {
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

  // A service a channel names, which this device must run.
  pub fn get(&self, name: &str) -> Result<Rc<Service>> {
    self.find(name).ok_or_else(|| missing(name))
  }

  // Refuse a new service of a package this device cannot run.
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

// A service's timeout, from the milliseconds its service.toml names.
pub(crate) fn timeout(milliseconds: Option<u64>) -> Duration {
  milliseconds.map_or(DEFAULT_TIMEOUT, Duration::from_millis)
}

// The version of an op made on this device for a service it does not run. No
// handler sees it: the op is refused when its turn comes, which also keeps it
// from running again and from being published.
pub const SERVICE_MISSING: &str = "service missing";

// Where a service a channel names leads when this device does not run it.
pub fn missing(name: &str) -> crate::error::ServiceError {
  not_found(format!("Service '{name}' is not registered on this device")).with_details(json!({"service": name}))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn service(name: &str) -> Service {
    replicated(name, Some("db"))
  }

  fn replicated(name: &str, binding: Option<&str>) -> Service {
    Service::replicated(name, "v1", binding, DEFAULT_TIMEOUT, Rc::new(|_: Rc<dyn Db>| Ok(())), Rc::new(|_| {}))
  }

  #[test]
  fn starts_with_the_builtins_and_registers_each_name_once() {
    let services = Services::builtin().unwrap();
    let builtins: Vec<(String, bool, Option<String>)> =
      ["carbon", "channel", "identity"].iter().map(|name| services.get(name).unwrap()).map(|service| (service.name.clone(), service.local, service.binding.clone())).collect();
    let db = || Some("db".to_string());
    assert_eq!(builtins, [("carbon".into(), true, None), ("channel".into(), false, db()), ("identity".into(), false, db())]);
    services.register(service("chat")).unwrap();
    for name in ["chat", "channel"] {
      assert_eq!(services.register(service(name)).err().unwrap().status, 409, "{name}");
    }
    assert_eq!(services.version("chat"), "v1");
  }

  #[test]
  fn names_what_is_missing() {
    let services = Services::builtin().unwrap();
    assert!(services.require_registered(&["channel".into()]).is_ok());
    let error = services.require_registered(&["channel".into(), "crm".into()]).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"services": ["crm"]}))));
    assert_eq!(services.get("crm").err().unwrap().details, Some(json!({"service": "crm"})));
  }
}
