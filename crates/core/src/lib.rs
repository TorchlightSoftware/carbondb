//! CarbonDB's core, single-node: spaces on SQLite, the services this device
//! runs, and the one path every call takes to them. Bindings for a host
//! language sit on top.

pub mod builtins;
pub mod call;
pub mod carbon;
pub mod clock;
pub mod config;
pub mod dispatch;
pub mod error;
pub mod hash;
pub mod identity;
pub mod invocation;
pub mod keys;
pub mod names;
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

pub use call::{Actor, Call, Settled};
pub use carbon::Carbon;
pub use error::{Result, ServiceError};
pub use identity::Identity;
pub use invocation::Invocation;
pub use services::{Entry, Service};
pub use space::Space;
pub use store::Db;
