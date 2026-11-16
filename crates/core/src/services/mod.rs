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
//! overrides the canonical name in its service.toml. A package is registered
//! as soon as it is read, so ops on it carry its version, but it is ready
//! only once its component has compiled: until then no space sets it up.

pub mod carbon;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use serde_json::json;

use crate::error::{Result, bad_request, conflict, not_found};
use crate::calls::service_call::ServiceCall;
use crate::{Db, HandlerContext};

// The packages CarbonDB ships, embedded in the library as the components
// `scripts/build-components` built and packaged from `crates/services/`. Each
// is versioned by the hash of its bytes, so devices on different releases of
// CarbonDB agree whenever a builtin is unchanged.
pub const BUILTINS: [(&str, &[u8]); 2] = [("channel", include_bytes!("../../builtins/channel.wasm")), ("identity", include_bytes!("../../builtins/identity.wasm"))];

// How long a call to a service may take to settle when its service.toml names no timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

// How long each space of a service may take to start when its service.toml names no time.
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_millis(10000);

// A service's startup timeout, from the milliseconds its service.toml names.
pub(crate) fn startup_timeout(milliseconds: Option<u64>) -> Duration {
  milliseconds.map_or(DEFAULT_STARTUP_TIMEOUT, Duration::from_millis)
}

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
  // Its health check, a path its spaces answer a GET on once set up before
  // they are ready; none, ready once set up.
  pub health: Option<String>,
  // How long each of its spaces may take to start: its package compiling, its
  // setup and its health check. Its calls' own deadlines start once it has.
  pub startup: Duration,
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
      health: None,
      startup: DEFAULT_STARTUP_TIMEOUT,
      setup,
      entry: Rc::new(move |call: Rc<ServiceCall>| entry(call)),
    }
  }
}

#[derive(Default)]
pub struct Services {
  registered: RefCell<BTreeMap<String, Rc<Service>>>,
  // The registered services whose component is still compiling.
  compiling: RefCell<BTreeSet<String>>,
}

impl Services {
  // What every CarbonDB starts with: `[@carbon]`; the builtin packages are
  // registered as they compile, as an app's are.
  pub fn builtin() -> Result<Services> {
    let services = Services::default();
    services.register(carbon::service())?;
    Ok(services)
  }

  // Register a service ready to run.
  pub fn register(&self, service: Service) -> Result<Rc<Service>> {
    let mut registered = self.registered.borrow_mut();
    if registered.contains_key(&service.name) {
      return Err(conflict(format!("A service '{}' is already registered on this device", service.name)));
    }
    let service = Rc::new(service);
    registered.insert(service.name.clone(), Rc::clone(&service));
    Ok(service)
  }

  // Register a service whose component is still compiling.
  pub fn register_compiling(&self, service: Service) -> Result<Rc<Service>> {
    let name = service.name.clone();
    let service = self.register(service)?;
    self.compiling.borrow_mut().insert(name);
    Ok(service)
  }

  // A service's component has compiled: it is ready, or, failing, not registered after all.
  pub fn compiled(&self, name: &str, result: &Result<()>) {
    self.compiling.borrow_mut().remove(name);
    if result.is_err() {
      self.registered.borrow_mut().remove(name);
    }
  }

  // Whether a service is registered but still compiling.
  pub fn is_compiling(&self, name: &str) -> bool {
    self.compiling.borrow().contains(name)
  }

  // A registered service, compiled or still compiling.
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
  fn starts_with_carbon_and_registers_each_name_once() {
    let services = Services::builtin().unwrap();
    let carbon = services.get("carbon").unwrap();
    assert_eq!((carbon.local, carbon.binding.clone()), (true, None));
    services.register(service("chat")).unwrap();
    for name in ["chat", "carbon"] {
      assert_eq!(services.register(service(name)).err().unwrap().status, 409, "{name}");
    }
    assert_eq!(services.version("chat"), "v1");
  }

  #[test]
  fn a_service_still_compiling_is_registered_but_not_ready_and_is_dropped_if_it_fails() {
    let services = Services::builtin().unwrap();
    for name in ["chat", "crm"] {
      services.register_compiling(service(name)).unwrap();
    }
    assert!(services.require_registered(&["chat".into()]).is_ok());
    assert_eq!((services.version("chat"), services.is_compiling("chat")), ("v1".into(), true), "ops on it carry its version");
    services.compiled("chat", &Ok(()));
    assert_eq!((services.find("chat").is_some(), services.is_compiling("chat")), (true, false));
    services.compiled("crm", &Err(bad_request("not a component")));
    assert_eq!((services.find("crm").is_none(), services.is_compiling("crm")), (true, false));
  }

  #[test]
  fn names_what_is_missing() {
    let services = Services::builtin().unwrap();
    services.register(service("channel")).unwrap();
    assert!(services.require_registered(&["channel".into()]).is_ok());
    let error = services.require_registered(&["channel".into(), "crm".into()]).unwrap_err();
    assert_eq!((error.status, error.details), (400, Some(json!({"services": ["crm"]}))));
    assert_eq!(services.get("crm").err().unwrap().details, Some(json!({"service": "crm"})));
  }
}
