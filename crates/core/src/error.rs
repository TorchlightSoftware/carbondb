//! What a call fails with, shared with every service through ServiceBindings.

pub use carbondb_service_bindings::error::*;

use rusqlite::ErrorCode;

// A failing SQLite statement is unexpected, so it surfaces as a 500 with
// SQLite's message, unless the authorizer refused it: then a service reached
// outside its own tables. A system failure is a 503: the space's file or disk
// failed, not the call.
pub(crate) fn sqlite(error: rusqlite::Error) -> ServiceError {
  let message = error.to_string();
  if is_system_failure(&error) {
    return system_failure(message);
  }
  if message.contains("not authorized") || message.contains("prohibited") {
    return forbidden(format!("A service reaches only its own tables, and SQLite refused this: {message}"));
  }
  internal(message)
}

// Whether SQLite failed in itself, whatever the statement: I/O, a full disk, a
// file busy or locked by another process, corruption, or memory. A space that
// meets one stops until it is fixed by hand and resumed.
pub(crate) fn is_system_failure(error: &rusqlite::Error) -> bool {
  matches!(
    error.sqlite_error_code(),
    Some(
      ErrorCode::SystemIoFailure
        | ErrorCode::DiskFull
        | ErrorCode::DatabaseBusy
        | ErrorCode::DatabaseLocked
        | ErrorCode::DatabaseCorrupt
        | ErrorCode::NotADatabase
        | ErrorCode::CannotOpen
        | ErrorCode::OutOfMemory
        | ErrorCode::FileLockingProtocolFailed
        | ErrorCode::NoLargeFileSupport
    )
  )
}

pub(crate) fn system_failure(message: impl Into<String>) -> ServiceError {
  ServiceError::new(503, "SYSTEM_FAILURE", message)
}

// What a call to a stopped space fails with: a 503 carrying the failure that stopped it.
pub(crate) fn stopped(space: &str, failure: &ServiceError) -> ServiceError {
  ServiceError::new(503, "SPACE_STOPPED", format!("Space {space} stopped after a system failure, and applies nothing until it is resumed: {}", failure.message))
    .with_details(serde_json::json!({"failure": failure}))
}

#[cfg(test)]
mod tests {
  use super::*;
  use rusqlite::ffi;

  fn failing(code: i32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)
  }

  #[test]
  fn classifies_sqlite_failing_in_itself_as_a_system_failure() {
    let cases = [
      (ffi::SQLITE_IOERR, 503),
      (ffi::SQLITE_FULL, 503),
      (ffi::SQLITE_BUSY, 503),
      (ffi::SQLITE_LOCKED, 503),
      (ffi::SQLITE_CORRUPT, 503),
      (ffi::SQLITE_NOTADB, 503),
      (ffi::SQLITE_CONSTRAINT, 500),
      (ffi::SQLITE_READONLY, 500),
      (ffi::SQLITE_ERROR, 500),
    ];
    for (code, status) in cases {
      assert_eq!(sqlite(failing(code)).status, status, "{code}");
    }
  }
}
