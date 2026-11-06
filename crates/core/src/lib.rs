//! CarbonDB's core, single-node: spaces on SQLite, the services this device
//! runs, and the one path every call takes to them. Services are written
//! against ServiceBindings (`carbondb-service-bindings`); bindings for a host
//! language sit on top.

pub mod call;
pub mod call_window;
pub mod carbon;
pub mod clock;
pub mod config;
pub mod dispatch;
pub mod error;
pub mod keys;
pub mod namespace;
pub mod op;
pub mod op_writer;
pub mod oplog;
pub mod results;
pub mod schema;
pub mod service_call;
pub mod services;
pub mod space;
pub mod store;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use call::Call;
pub use carbon::Carbon;
pub use carbondb_service_bindings::hash;
pub use carbondb_service_bindings::{Actor, Db, HandlerContext, Op, Package, Reply};
pub use dispatch::Settled;
pub use error::{Result, ServiceError};
pub use services::carbon::Identity;
pub use services::{Entry, Init, Service};
pub use space::{RunState, Space};
