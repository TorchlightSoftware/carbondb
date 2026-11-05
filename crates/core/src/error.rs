//! What a call fails with: the same shape as `@tlsft/service-error`'s data, so
//! an error raised in the core reaches a JS caller as the same service error.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceError {
  pub status: u16,
  pub code: String,
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<Value>,
}

pub type Result<T> = std::result::Result<T, ServiceError>;

impl ServiceError {
  pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
    ServiceError { status, code: code.to_string(), message: message.into(), details: None }
  }

  pub fn with_details(mut self, details: Value) -> Self {
    self.details = Some(details);
    self
  }
}

pub fn bad_request(message: impl Into<String>) -> ServiceError {
  ServiceError::new(400, "BAD_REQUEST", message)
}

pub fn forbidden(message: impl Into<String>) -> ServiceError {
  ServiceError::new(403, "FORBIDDEN", message)
}

pub fn not_found(message: impl Into<String>) -> ServiceError {
  ServiceError::new(404, "NOT_FOUND", message)
}

pub fn conflict(message: impl Into<String>) -> ServiceError {
  ServiceError::new(409, "CONFLICT", message)
}

pub fn internal(message: impl Into<String>) -> ServiceError {
  ServiceError::new(500, "INTERNAL_ERROR", message)
}

// A call that needs peers, refused because this CarbonDB runs single-node.
pub fn single_node(what: &str) -> ServiceError {
  ServiceError::new(501, "SINGLE_NODE", format!("{what} needs peers, and this CarbonDB runs single-node"))
}

// Something designed but not built yet, such as replicated mode.
pub fn not_built(what: &str) -> ServiceError {
  ServiceError::new(501, "NOT_BUILT", format!("{what} is not built yet"))
}

impl fmt::Display for ServiceError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(formatter, "{} {}: {}", self.status, self.code, self.message)
  }
}

impl std::error::Error for ServiceError {}

// A failing SQLite statement is unexpected, so it surfaces as a 500 with SQLite's message.
impl From<rusqlite::Error> for ServiceError {
  fn from(error: rusqlite::Error) -> Self {
    internal(error.to_string())
  }
}

impl From<serde_json::Error> for ServiceError {
  fn from(error: serde_json::Error) -> Self {
    bad_request(error.to_string())
  }
}
