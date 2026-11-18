//! Creating channels and services, which only `[@carbon]` does, by effects
//! of its own ops: `POST [@carbon]/services` and `POST [@carbon]/channel` on
//! a channel's handle, and founding an identity (`identities`).
//!
//! The `create` effect makes the new channel or service here and answers its
//! id. A new channel is a p2panda space when this device syncs, whose id is
//! the channel's, with its log and `[channel]` store; single-node, and for a
//! service's store always, the id derives from the op. `[@carbon]` then names
//! what it created by the channel's own calls, which replicate: `POST
//! [channel]/services`, signed with the op's device key by the `sign` effect,
//! and `POST [channel]/ref`. A `[@carbon]` op replays on every replica of its
//! channel like any op, its effects' recorded outputs with it, so a replica
//! never creates anything again: what it must hold follows from its
//! channel's committed state (`crate::nodes::load`). A refused op's first run
//! deletes what its `create` effects made.

use std::collections::VecDeque;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};

use super::acting_identity;
use super::steps::{self, Step};
use crate::calls::service_call::ServiceCall;
use crate::channels::service::Runs;
use crate::config::SyncMode;
use crate::error::{Result, bad_request, internal};
use crate::{HandlerContext, Reply, Routing, keys};
use carbondb_service_bindings::signature::claim;
use carbondb_svc_channel::Named;

// The `create` effect: a new channel, for the identity named or acting, or a
// new service's store in the caller's channel, created here. It answers the
// id, the version of the package it runs, and whether it is a channel.
pub(crate) fn create_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
  let package = input["package"].as_str().unwrap_or_default().to_string();
  let name = input["name"].as_str().map_or_else(|| package.clone(), str::to_string);
  let version = caller.carbon().services().version(&package);
  if package != "channel" {
    return reply(derived_id(&caller, &input).and_then(|id| create_service(&caller, &id, package, name)).map(|id| json!({"space": id, "version": version})));
  }
  let identity = input["identity"].as_str().map(str::to_string).or_else(|| caller.actor.identity.clone()).unwrap_or_default();
  let answer = move |id: String| json!({"space": id, "version": version, "channel": true});
  if caller.carbon().config.sync == SyncMode::None {
    return reply(derived_id(&caller, &input).and_then(|id| caller.carbon().create_channel(&id, &identity).map(|_| answer(id))));
  }
  let creating = Rc::clone(&caller);
  caller.carbon().create_p2panda_space(&identity.clone(), Box::new(move |created| match created {
    Ok(id) if !creating.is_live() => {
      creating.carbon().unfollow(&id);
      reply(Err(internal(format!("{} ended before its channel was created", creating.call.key()))))
    }
    Ok(id) => reply(creating.carbon().create_channel(&id, &identity).map(|_| answer(id))),
    Err(error) => reply(Err(error)),
  }));
}

// What a single-node channel, or any service's store, is named: the same on
// every run of the op that creates it.
fn derived_id(caller: &ServiceCall, input: &Value) -> Result<String> {
  let op = caller.op().ok_or_else(|| bad_request("Creating needs an op"))?;
  Ok(crate::hash::hash(&json!({"op": op.hash, "create": input})))
}

fn create_service(caller: &Rc<ServiceCall>, id: &str, package: String, name: String) -> Result<String> {
  let channel = Rc::clone(&caller.space().channel);
  caller.carbon().create_service(&channel, id, Runs { package, name })?;
  Ok(id.to_string())
}

// The `sign` effect: what a service records only on `[@carbon]`'s word,
// signed with the acting identity's device key on this device.
pub(crate) fn sign_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
  let signed = acting_identity(&caller).and_then(|identity| caller.carbon().credentials_of(&identity));
  reply(signed.map(|credentials| json!(keys::sign(&credentials, &claim(input["key"].as_str().unwrap_or_default(), &input["recorded"])))))
}

// A service a channel names: the package registered on this device that runs
// it, which defaults to the name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewService {
  name: String,
  package: Option<String>,
}

// `POST [@carbon]/services`: a service the channel names, its store created
// here first, then named in `[channel]`, signed. Answers what `[channel]` answers.
pub fn add_service(call: Rc<ServiceCall>) {
  let request: NewService = match serde_json::from_value(call.input.clone()) {
    Ok(request) => request,
    Err(error) => return settle(&call, Err(error.into())),
  };
  let package = request.package.unwrap_or_else(|| request.name.clone());
  let caller = Rc::clone(&call);
  HandlerContext::effect(call, "create", json!({"package": package, "name": request.name}), Box::new(move |created| {
    let named = created.map(|created| Named { name: request.name, service: text(&created, "space"), package, version: text(&created, "version") });
    match named {
      Ok(named) => name_service(caller, named),
      Err(error) => settle(&caller, Err(error)),
    }
  }));
}

// Name a service in the channel, signed by the `sign` effect.
fn name_service(call: Rc<ServiceCall>, named: Named) {
  let recorded = carbondb_svc_channel::signed(&named);
  let caller = Rc::clone(&call);
  HandlerContext::effect(call, "sign", json!({"key": "POST [channel]/services", "recorded": recorded}), Box::new(move |signature| {
    let input = signature.map(|signature| json!({"name": named.name, "space": named.service, "package": named.package, "version": named.version, "signature": signature}));
    match input {
      Ok(input) => {
        let answering = Rc::clone(&caller);
        HandlerContext::call(caller, "POST [channel]/services", input, Box::new(move |named| settle(&answering, named)))
      }
      Err(error) => settle(&caller, Err(error)),
    }
  }));
}

// A channel to create inside this one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewChannel {
  #[serde(default = "default_kind")]
  kind: String,
  name: String,
  manage: Option<Vec<String>>,
  slug: Option<String>,
  #[serde(default)]
  cascade: Vec<String>,
}

fn default_kind() -> String {
  "channel".to_string()
}

// `POST [@carbon]/channel`: a channel nested in this one, created here
// first, then pointed to from this channel, then its origin written there.
// Answers the ref.
pub fn add_channel(call: Rc<ServiceCall>) {
  let planned = (|| -> Result<(NewChannel, String)> { Ok((serde_json::from_value(call.input.clone())?, acting_identity(&call)?)) })();
  let (request, identity) = match planned {
    Ok(planned) => planned,
    Err(error) => return settle(&call, Err(error)),
  };
  let caller = Rc::clone(&call);
  HandlerContext::effect(call, "create", json!({"package": "channel", "name": "channel"}), Box::new(move |created| {
    let created = match created {
      Ok(created) => text(&created, "space"),
      Err(error) => return settle(&caller, Err(error)),
    };
    let reference = json!({"channel": created, "kind": request.kind, "name": request.name, "slug": request.slug, "cascade": request.cascade});
    let origin = Step { space: created, actor: caller.actor.clone(), key: "POST [channel]/origin", input: json!({"kind": request.kind, "name": request.name, "manage": request.manage.unwrap_or_else(|| vec![identity])}) };
    let answering = Rc::clone(&caller);
    HandlerContext::call(Rc::clone(&caller), "POST [channel]/ref", reference, Box::new(move |referred| match referred {
      Ok(referred) => {
        let finishing = Rc::clone(&answering);
        steps::run(answering, VecDeque::from([origin]), Box::new(move |written| settle(&finishing, written.map(|_| referred))))
      }
      Err(error) => settle(&answering, Err(error)),
    }));
  }));
}

fn text(value: &Value, field: &str) -> String {
  value[field].as_str().unwrap_or_default().to_string()
}

fn settle(call: &ServiceCall, result: Result<Value>) {
  let _ = HandlerContext::settle(call, result);
}
