//! Ops: a service call that writes, stamped (`op`) with its time from this
//! device's clock (`clock`), written to its space's op log (`op_writer`,
//! `oplog`), applied in the shared order with what became of it kept as its
//! result (`results`), and undone and re-run when an op arrives late
//! (`rewind`). What an op does outside its service's own tables are its
//! effects (`effects`), made on its first run and replayed after. How replicas
//! order, rewind and replay ops is told in docs/replication.md.

pub mod clock;
pub mod effects;
pub mod op;
pub mod op_writer;
pub mod oplog;
pub mod results;
pub mod rewind;
