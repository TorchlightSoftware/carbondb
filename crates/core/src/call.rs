//! A call as the core receives it: the service key already parsed into parts by
//! the runtime, the acting identity, and what the call produced.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, not_found};
use crate::store::Store;

// A service key's parts: `POST [space]/ref` is {method: POST, service: space, resource: /ref}.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Call {
  pub method: Option<String>,
  pub service: String,
  pub resource: String,
  #[serde(default)]
  pub confined: bool,
}

impl Call {
  pub fn method(&self) -> &str {
    self.method.as_deref().unwrap_or("")
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
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Actor {
  pub identity: Option<String>,
  pub device: Option<String>,
}

// What a call produced, and the tables it read or wrote, for watch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
  pub value: Value,
  pub tables: Vec<String>,
  pub wrote: bool,
}

impl Outcome {
  pub fn read(store: &Store, work: impl FnOnce(&Store) -> Result<Value>) -> Result<Outcome> {
    let (value, tables) = store.read(work)?;
    Ok(Outcome { value, tables, wrote: false })
  }

  pub fn write(store: &Store, work: impl FnOnce(&Store) -> Result<Value>) -> Result<Outcome> {
    let (value, tables) = store.write(work)?;
    Ok(Outcome { value, tables, wrote: true })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn formats_its_key() {
    let cases = [
      (Call { method: Some("POST".into()), service: "space".into(), resource: "/ref".into(), confined: false }, "POST [space]/ref"),
      (Call { method: Some("GET".into()), service: "carbon".into(), resource: "/status".into(), confined: true }, "GET [@carbon]/status"),
      (Call { method: None, service: "device".into(), resource: "/refs".into(), confined: false }, "[device]/refs"),
    ];
    for (call, key) in cases {
      assert_eq!(call.key(), key);
    }
  }
}
