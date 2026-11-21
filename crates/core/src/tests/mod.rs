//! Tests across the core: how calls apply, nest and roll back; what the op log
//! and results record; what `[@carbon]` founds, through `[channel]` and
//! `[identity]`, as ops; how channels name and create services; how effects
//! are made once and replayed; and how replicas converge.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::ops::results::{self, Outcome};
use crate::testing::{actor, alice, call_now, carbon, entries, now, on, room, room_channel, service, space_now, texts};
use crate::ops::oplog;
use crate::{Actor, Db, Routing};

// The keys of the ops addressed to a store, in order.
fn logged(space: &crate::ServiceStore) -> Vec<String> {
  entries(space).into_iter().map(|op| op.key).collect()
}

// What became of each op addressed to a store, in order: applied, or refused with its status.
fn outcomes(space: &crate::ServiceStore) -> Vec<Result<(), u16>> {
  let ops = entries(space);
  ops.iter().map(|op| match results::of(space.store(), &op.hash).unwrap() {
    Some(Outcome::Applied { .. }) => Ok(()),
    Some(Outcome::Refused { error, .. }) => Err(error.status),
    None => Err(0),
  }).collect()
}

// A row from alice's other device, at a time of its own, received here.
fn arrives(carbon: &Rc<crate::Carbon>, space: &crate::ServiceStore, script: Value, time: u64) {
  let call = crate::Call::from_key(&format!("POST [{}]/run", space.runs.name)).unwrap();
  let actor = Actor { identity: Some("alice".into()), device: Some("laptop".into()) };
  let op = crate::ops::op::stamp(&actor, &call, &json!({"do": script}), &carbon.services().version(&space.runs.package), time);
  carbon.receive_in(&carbon.opened(&space.id).unwrap(), oplog::LogRow::new(op)).unwrap();
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
mod causes;
mod reads;
