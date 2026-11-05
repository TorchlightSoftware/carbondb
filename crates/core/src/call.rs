//! A call as the core receives it: the service key parsed into parts, the
//! acting identity, and what the call settled with.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, bad_request, not_found};

// A service key's parts: `POST [space]/ref` is {method: POST, service: space, resource: /ref}.
// A confined key, `[@carbon]`, names a local service.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Call {
  pub method: Option<String>,
  pub service: String,
  pub resource: String,
  #[serde(default)]
  pub confined: bool,
}

impl Call {
  // A call from its service key, for calls the core makes itself. Apps' keys
  // are parsed by the runtime, which knows the full syntax.
  pub fn from_key(key: &str) -> Result<Call> {
    let malformed = || bad_request(format!("'{key}' is not a service key"));
    let (method, rest) = key.split_once(' ').ok_or_else(malformed)?;
    let (service, resource) = rest.strip_prefix('[').and_then(|rest| rest.split_once(']')).ok_or_else(malformed)?;
    let confined = service.starts_with('@');
    Ok(Call { method: Some(method.into()), service: service.trim_start_matches('@').into(), resource: resource.into(), confined })
  }

  pub fn method(&self) -> &str {
    self.method.as_deref().unwrap_or("")
  }

  // A GET reads committed state and is never an op; every other call writes.
  pub fn writes(&self) -> bool {
    self.method() != "GET"
  }

  pub fn key(&self) -> String {
    let at = if self.confined { "@" } else { "" };
    format!("{} [{at}{}]{}", self.method(), self.service, self.resource).trim_start().to_string()
  }

  pub fn unknown(&self) -> crate::error::ServiceError {
    not_found(format!("{} is not a call CarbonDB answers", self.key()))
  }
}

// Who is acting: the identity a handle acts for, and this device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
  pub identity: Option<String>,
  pub device: Option<String>,
}

// What a call settled with, and the tables it read or wrote, for watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settled {
  pub value: Value,
  pub tables: Vec<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn formats_and_parses_its_key() {
    let cases = [
      (Call { method: Some("POST".into()), service: "space".into(), resource: "/ref".into(), confined: false }, "POST [space]/ref"),
      (Call { method: Some("GET".into()), service: "carbon".into(), resource: "/status".into(), confined: true }, "GET [@carbon]/status"),
    ];
    for (call, key) in cases {
      assert_eq!(call.key(), key);
      assert_eq!(Call::from_key(key).unwrap(), call);
    }
    let unmethoded = Call { method: None, service: "device".into(), resource: "/refs".into(), confined: false };
    assert_eq!(unmethoded.key(), "[device]/refs");
    assert_eq!(Call::from_key("not a key").unwrap_err().status, 400);
  }
}
