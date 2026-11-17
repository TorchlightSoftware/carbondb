//! Tests across the core: how calls apply, nest and roll back; what the op log
//! and results record; what `[@carbon]` founds, through `[channel]` and
//! `[identity]`, as ops; how channels name and create services; how effects
//! are made once and replayed; and how replicas converge.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::ops::results::{self, Outcome};
use crate::testing::{actor, alice, call_now, carbon, now, on, room, room_channel, service, space_now, texts};
use crate::ops::oplog;
use crate::{Actor, Db, Routing};

// The keys of a space's op log, in order.
fn logged(space: &crate::ServiceSpace) -> Vec<String> {
  oplog::entries(space.store()).unwrap().into_iter().map(|op| op.key).collect()
}

// What became of each op in a space's log, in order: applied, or refused with its status.
fn outcomes(space: &crate::ServiceSpace) -> Vec<Result<(), u16>> {
  let ops = oplog::entries(space.store()).unwrap();
  ops.iter().map(|op| match results::of(space.store(), &op.hash).unwrap() {
    Some(Outcome::Applied { .. }) => Ok(()),
    Some(Outcome::Refused { error, .. }) => Err(error.status),
    None => Err(0),
  }).collect()
}

mod transactions;
mod oplog_entries;
mod run_states;
mod identities;
mod channels;
mod effects;
mod deadlines;
mod components;
mod lifecycle;
mod fetch;
mod replicas;
mod nodes;
mod milestone;
