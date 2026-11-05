//! CarbonDB's core, single-node: spaces on SQLite, identities and their
//! devices, and the builtin services. Bindings for a host language sit on top.

pub mod builtins;
pub mod call;
pub mod carbon;
pub mod clock;
pub mod config;
pub mod error;
pub mod hash;
pub mod identity;
pub mod keys;
pub mod names;
pub mod queue;
pub mod space;
pub mod store;
#[cfg(test)]
mod testing;

pub use call::{Actor, Call, Outcome};
pub use carbon::{Carbon, Context, Stamp};
pub use error::{Result, ServiceError};
pub use identity::Identity;
pub use queue::Queue;
pub use store::{Side, Store};
