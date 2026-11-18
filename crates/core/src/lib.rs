//! CarbonDB's core, single-node: each service in its own SQLite file, in a
//! channel; the services this device runs, and the one path every call takes
//! to them. Services are written against ServiceBindings
//! (`carbondb-service-bindings`); bindings for a host language sit on top.

pub mod app;
pub mod calls;
pub mod carbon;
pub mod channels;
pub mod config;
pub mod error;
pub mod keys;
pub mod nodes;
pub mod ops;
pub mod services;
pub mod sql;
pub mod store;
pub mod wasm;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use calls::call::{Call, Routing};
pub use carbon::{Cancel, Carbon, Schedule, Spawn, Task};
pub use carbondb_service_bindings::hash;
pub use carbondb_service_bindings::{Actor, Db, HandlerContext, Op, Reply};
pub use calls::dispatch::Settled;
pub use app::runtime::Runtime;
pub use error::{Result, ServiceError};
pub use services::carbon::Identity;
pub use services::{Entry, Service, Setup};
pub use channels::service::{RunState, ServiceStore};
