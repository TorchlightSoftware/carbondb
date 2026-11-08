//! A service key parsed into the parts the core routes on: `POST [space]/ref`
//! is {method: POST, service: space, resource: /ref}. The grammar is the
//! `service-key` crate's, the same as the JS runtime's, so a key means the
//! same wherever it is written.

use crate::error::{Result, ServiceError, bad_request, not_found};

// A call's parts. A confined key, `[@carbon]`, names a local service.
pub use service_key::ServiceKey as Call;

// What the core reads from a call's parts.
pub trait Routing: Sized {
  // A call from its service key, or a refusal naming what is not one.
  fn from_key(key: &str) -> Result<Self>;

  fn method(&self) -> &str;

  // A GET reads committed state and is never an op; every other call writes.
  fn writes(&self) -> bool;

  fn key(&self) -> String;

  // The refusal for a method and resource a service does not answer.
  fn unknown(&self) -> ServiceError;
}

impl Routing for Call {
  fn from_key(key: &str) -> Result<Call> {
    Call::parse(key).ok_or_else(|| bad_request(format!("'{key}' is not a service key")))
  }

  fn method(&self) -> &str {
    self.method.as_deref().unwrap_or("")
  }

  fn writes(&self) -> bool {
    self.method() != "GET"
  }

  fn key(&self) -> String {
    self.to_string()
  }

  fn unknown(&self) -> ServiceError {
    not_found(format!("{} is not a call CarbonDB answers", self.key()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_what_it_routes_on_from_a_key() {
    let cases = [("POST [space]/ref", "POST", true), ("GET [@carbon]/status", "GET", false), ("[device]/refs", "", true)];
    for (key, method, writes) in cases {
      let call = Call::from_key(key).unwrap();
      assert_eq!((call.method(), call.writes(), call.key()), (method, writes, key.to_string()), "{key}");
    }
    assert_eq!(Call::from_key("not a key").unwrap_err().status, 400);
  }
}
