//! Ops: a service call that writes, stamped (`op`) with its time from this
//! device's clock (`clock`), written to its channel's log (`oplog`) and
//! pointed to by the service it is addressed to (`pointers`, `op_writer`),
//! applied in the shared order with what became of it kept as its result
//! (`results`), and undone and re-run when an op arrives late, or when its
//! cause fails (`rewind`). What an op does outside its service's own tables are its
//! effects (`effects`), made on its first run and replayed after, an HTTP request
//! to the outside world among them (`fetch`). How replicas
//! order, rewind and replay ops is told in docs/replication.md.

pub mod clock;
pub mod effects;
pub mod errors;
pub mod fetch;
pub mod op;
pub mod op_writer;
pub mod pointers;
pub mod oplog;
pub mod published;
pub mod results;
pub mod rewind;
