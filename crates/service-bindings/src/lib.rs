//! ServiceBindings: the toolkit a Rust CarbonDB service builds with.
//!
//! At runtime, the API footprint a service exposes, no less and no more. A
//! service knows nothing of the runtime that runs it, whether CarbonDB, Node.js
//! or a Worker; the runtime invokes it.
//!
//! A service exposes its name and version, an `init` the runtime calls to set
//! up its schema, and one entry point. The entry point receives each service
//! call as a handler context: the call's method, resource, input and actor;
//! the `db` binding the service asked for; `call` to make calls of its own;
//! and the op, when the call writes. It settles the context once.
//!
//! `migrations` is an opt-in helper for `init`, not part of the footprint.
//! `hash` is the content hash every replica agrees on.
//!
//! At build time, with the `build` feature, `build` packages the service: its
//! build script calls `build::package()`.

#[cfg(feature = "build")]
pub mod build;
pub mod context;
pub mod error;
pub mod hash;
pub mod migrations;
pub mod op;

pub use context::{Actor, Db, HandlerContext, Package, Reply};
pub use error::{Result, ServiceError};
pub use op::Op;
