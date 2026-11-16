//! ServiceBindings: the toolkit a Rust CarbonDB service builds with.
//!
//! At runtime, the API footprint a service exposes, no less and no more. A
//! service knows nothing of the runtime that runs it, whether CarbonDB, Node.js
//! or a Worker; the runtime invokes it.
//!
//! A service exposes its name and version, a `setup` the runtime calls to set
//! up its schema, and one entry point. The entry point receives each service
//! call as a handler context: the call's method, resource, input and actor;
//! the `db` binding the service asked for; `call` to make calls of its own;
//! and the op, when the call writes. It settles the context once.
//!
//! `migrations` is an opt-in helper for `setup`, not part of the footprint.
//! `hash` is the content hash every replica agrees on.
//!
//! A service ships as a WebAssembly component of the `carbondb:service` world
//! (`wit/service.wit`): with the `guest` feature, `guest` exports a Rust
//! service as one, and `package` embeds its `service.toml` in it. With the
//! `manifest` feature, `manifest` reads a `service.toml`, and the
//! `carbondb-package` tool packages a built component.

pub mod context;
pub mod error;
#[cfg(feature = "guest")]
pub mod guest;
pub mod hash;
#[cfg(feature = "manifest")]
pub mod manifest;
pub mod migrations;
pub mod op;
pub mod package;

pub use context::{Actor, Db, HandlerContext, Package, Reply};
pub use error::{Result, ServiceError};
pub use op::Op;
