//! What a call fails with, shared with every service through the service API.

pub use carbondb_service_api::error::*;

// A failing SQLite statement is unexpected, so it surfaces as a 500 with SQLite's message.
pub(crate) fn sqlite(error: rusqlite::Error) -> ServiceError {
  internal(error.to_string())
}
