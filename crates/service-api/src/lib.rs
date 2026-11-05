//! What every CarbonDB service is written against, whatever runs it: the
//! invocation it answers, its queries, the calls and ops it sees, the errors it
//! fails with, the hash every replica agrees on, and space names. `build`
//! packages a Rust service's `service.toml` and migrations from the service's
//! build script.

pub mod build;
pub mod call;
pub mod error;
pub mod hash;
pub mod invocation;
pub mod names;
pub mod op;

pub use call::{Actor, Call};
pub use error::{Result, ServiceError};
pub use invocation::{Db, Entry, Invocation, Package, Reply, answer, ask, now};
pub use op::Op;
