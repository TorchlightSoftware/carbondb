//! Calls: a service key parsed into what the core routes on (`call`), the
//! one path every call takes (`dispatch`), a call running in the core as its
//! service's handler context (`service_call`), the deadline it settles within
//! (`deadline`), and the GETs that run again after every commit that writes
//! what they read (`watch`).

pub mod call;
pub mod deadline;
pub mod dispatch;
pub mod service_call;
pub mod watch;
