//! CarbonDB's core, single-node: spaces on SQLite, the services this device
//! runs, and the one path every call takes to them. Services are written
//! against `carbondb-service-api`; bindings for a host language sit on top.

pub mod carbon;
pub mod clock;
pub mod config;
pub mod dispatch;
pub mod error;
pub mod frame;
pub mod keys;
pub mod op;
pub mod oplog;
pub mod run_queue;
pub mod services;
pub mod space;
pub mod store;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use carbondb_service_api::{Actor, Call, Db, Entry, Invocation, Op, Package, ask, hash, names, now};
pub use carbon::Carbon;
pub use dispatch::Settled;
pub use error::{Result, ServiceError};
pub use services::Service;
pub use services::carbon::Identity;
pub use space::Space;
