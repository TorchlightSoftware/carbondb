//! CarbonDB's core, single-node: spaces on SQLite, identities and their
//! devices, and the builtin services. Bindings for a host language sit on top.

pub mod builtins;
pub mod call;
pub mod carbon;
pub mod clock;
pub mod config;
pub mod create;
pub mod error;
pub mod hash;
pub mod keys;
pub mod names;
pub mod space;
pub mod store;
#[cfg(test)]
mod testing;

pub use call::{Actor, Call, Outcome};
pub use carbon::{Carbon, Identity, Stamp};
pub use error::{Result, ServiceError};
pub use store::{Side, Store};
