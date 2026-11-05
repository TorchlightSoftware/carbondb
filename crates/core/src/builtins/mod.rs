//! The services CarbonDB itself runs, registered like any other: `[@carbon]`,
//! the device's local service, and the replicated `[space]`, `[identity]` and
//! `[device]`. Each is its migrations and an entry point that routes its own
//! calls; CarbonDB routes to it by service name only.

pub mod carbon;
pub mod device;
pub mod identity;
pub mod space;

use std::rc::Rc;

use serde_json::Value;

use crate::error::Result;
use crate::invocation::Invocation;
use crate::services::{Entry, Service, Services};

// The services every CarbonDB starts with.
pub fn services() -> Services {
  let services = Services::default();
  let builtins = [
    builtin("carbon", true, &[], Rc::new(carbon::entry)),
    builtin("space", false, space::MIGRATIONS, answering(space::handle)),
    builtin("identity", false, identity::MIGRATIONS, answering(identity::handle)),
    builtin("device", false, device::MIGRATIONS, answering(device::handle)),
  ];
  for service in builtins {
    services.register(service).expect("builtin names are distinct");
  }
  services
}

fn builtin(name: &str, local: bool, migrations: &[&str], entry: Entry) -> Service {
  Service { name: name.into(), local, migrations: migrations.iter().map(|sql| sql.to_string()).collect(), entry }
}

// The entry point of a service whose handlers answer at once.
fn answering(handle: fn(&Invocation) -> Result<Value>) -> Entry {
  Rc::new(move |invocation: Rc<Invocation>| {
    let result = handle(&invocation);
    let _ = invocation.settle(result);
  })
}
