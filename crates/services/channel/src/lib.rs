//! `[channel]`: what a channel is, the services it names and the channels it
//! points to, its members, their invitations, and its manage list. It runs in
//! every channel's own space.
//!
//! Every POST but the origin and leaving needs manage on the channel, and a
//! sealed channel refuses every such POST after its seal. What changes p2panda or reaches another
//! device (adding or removing a member, an invitation, joining, leaving, a
//! change of manage) is an effect; so is creating the space of a new channel
//! or service.

pub mod names;

use std::rc::Rc;

use carbondb_service_bindings::error::{Result, ServiceError, bad_request, conflict, forbidden, not_found, unknown};
use carbondb_service_bindings::hash::hash;
use carbondb_service_bindings::{Db, HandlerContext, Package, Reply, migrations};
use names::{slugify, unique_slug};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PACKAGE: Package = Package { setup, entry };

#[cfg(target_arch = "wasm32")]
carbondb_service_bindings::serve!(PACKAGE);

// Its tables, set up by `setup` on every open.
const MIGRATIONS: &[migrations::Migration] = &[("0001_channel", include_str!("../migrations/0001_channel.sql"))];

fn setup(db: &dyn Db) -> Result<()> {
  migrations::apply(db, "d1_migrations", MIGRATIONS)
}

// What a channel is, fixed by its first op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
  pub kind: String,
  pub name: String,
  pub manage: Vec<String>,
}

// A pointer from this channel to another. It grants nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
  pub channel: String,
  pub kind: String,
  pub name: String,
  pub slug: String,
  #[serde(default)]
  pub cascade: Vec<String>,
}

// A ref as a call asks for it: its slug defaults to the name's, and is made
// unique among this channel's refs of the same kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRef {
  channel: String,
  kind: String,
  name: String,
  slug: Option<String>,
  #[serde(default)]
  cascade: Vec<String>,
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

// A service to create in this channel: the name the channel calls it by, and
// the package registered on this device that runs it, which defaults to the name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewService {
  name: String,
  package: Option<String>,
}

// A service this channel names: `[name]` resolves to its space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Named {
  pub name: String,
  pub service: String,
  pub package: String,
  pub version: String,
}

// The ref a kind and slug name here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefName {
  kind: String,
  slug: String,
}

type Context = Rc<dyn HandlerContext>;

fn entry(context: Context) {
  let resource = context.resource().to_string();
  match (context.method(), resource.as_str()) {
    ("POST", "/origin") => {
      let written = serde_json::from_value(context.input().clone()).map_err(Into::into).and_then(|origin| write_origin(context.as_ref(), origin));
      settle(&context, written)
    }
    ("GET", "/origin") => settle(&context, origin(context.as_ref()).map(|origin| json!(origin))),
    ("GET", "/manage") => settle(&context, origin(context.as_ref()).map(|origin| json!(origin.manage))),
    ("GET", "/services") => settle(&context, services(context.as_ref()).map(|services| json!(services))),
    ("GET", "/refs") => settle(&context, refs(context.as_ref()).map(|refs| json!(refs))),
    ("GET", "/ref") => {
      let found = serde_json::from_value(context.input().clone()).map_err(Into::into).and_then(|RefName { kind, slug }| find_ref(context.as_ref(), &kind, &slug));
      settle(&context, found.and_then(|found| found.map(|found| json!(found)).ok_or_else(|| not_found("No such ref in this channel"))))
    }
    ("GET", "/members") => settle(&context, members(context.as_ref())),
    ("GET", "/invitations") => settle(&context, invitations(context.as_ref())),
    ("POST", "/leave") => leave(context),
    ("POST", _) | ("DELETE", _) => managing(context, resource),
    (method, resource) => settle(&context, Err(unknown(method, resource))),
  }
}

// A call that changes the channel: only a manager makes one, and none once it is sealed.
fn managing(context: Context, resource: String) {
  if let Err(error) = may_change(context.as_ref()) {
    return settle(&context, Err(error));
  }
  match (context.method(), resource.as_str()) {
    ("POST", "/seal") => settle(&context, seal(context.as_ref())),
    ("POST", "/ref") => {
      let added = serde_json::from_value(context.input().clone()).map_err(Into::into).and_then(|new| add_ref(context.as_ref(), new));
      settle(&context, added.map(|reference| json!(reference)))
    }
    ("POST", "/channel") => create_channel(context),
    ("POST", "/services") => create_service(context),
    ("POST", "/members") => invite(context),
    ("POST", "/join") => join(context),
    ("POST", "/manage") => change_manage(context),
    ("DELETE", resource) if resource.starts_with("/invitations/") => cancel(context, resource["/invitations/".len()..].to_string()),
    ("DELETE", resource) if resource.starts_with("/members/") => remove(context, resource["/members/".len()..].to_string()),
    (method, resource) => settle(&context, Err(unknown(method, resource))),
  }
}

fn settle(context: &Context, result: Result<Value>) {
  let _ = context.settle(result);
}

// Make an effect, then, if it succeeded, finish with what it answered.
fn after(context: &Context, name: &str, input: Value, then: impl FnOnce(&Context, Value) -> Result<Value> + 'static) {
  let caller = Rc::clone(context);
  let reply: Reply = Box::new(move |output| {
    let finished = output.and_then(|output| then(&caller, output));
    settle(&caller, finished)
  });
  Rc::clone(context).effect(name, input, reply)
}

fn json_column(row: &Value, column: &str) -> Result<Vec<String>> {
  Ok(serde_json::from_str(row[column].as_str().unwrap_or("[]"))?)
}

fn write_origin(db: &dyn Db, origin: Origin) -> Result<Value> {
  if !db.get("SELECT 1 AS present FROM origin", &[])?.is_null() {
    return Err(conflict("A channel's origin is written once"));
  }
  db.run("INSERT INTO origin (one, kind, name, manage) VALUES (1, ?, ?, ?)", &[json!(origin.kind), json!(origin.name), json!(origin.manage)])?;
  for identity in &origin.manage {
    db.run("INSERT OR REPLACE INTO member (identity, access) VALUES (?, 'manage')", &[json!(identity)])?;
  }
  Ok(json!(origin))
}

fn origin(db: &dyn Db) -> Result<Origin> {
  let row = db.get("SELECT kind, name, manage FROM origin", &[])?;
  if row.is_null() {
    return Err(not_found("This channel has no origin yet"));
  }
  Ok(Origin { kind: row["kind"].as_str().unwrap_or_default().to_string(), name: row["name"].as_str().unwrap_or_default().to_string(), manage: json_column(&row, "manage")? })
}

// Whether the acting identity may change the channel: a manager, while it is not sealed.
fn may_change(context: &dyn HandlerContext) -> Result<()> {
  let actor = context.actor().identity.clone().ok_or_else(|| forbidden("Changing a channel needs an acting identity"))?;
  let row = context.get("SELECT manage, sealed FROM origin", &[])?;
  if row.is_null() {
    return Err(not_found("This channel has no origin yet"));
  }
  if row["sealed"] == json!(1) {
    return Err(forbidden("This channel is sealed and takes no further changes"));
  }
  match json_column(&row, "manage")?.contains(&actor) {
    true => Ok(()),
    false => Err(forbidden("Only a manager of this channel may change it")),
  }
}

fn seal(db: &dyn Db) -> Result<Value> {
  db.run("UPDATE origin SET sealed = 1", &[])?;
  Ok(json!({"sealed": true}))
}

// The id of a channel or service this op creates: the same on every replica.
fn new_id(context: &dyn HandlerContext, what: &str, name: &str) -> Result<String> {
  let op = context.op().ok_or_else(|| bad_request("Creating needs an op"))?;
  Ok(hash(&json!({"op": op.hash, what: name})))
}

// A channel inside this one: its space is created, its origin written there,
// and this channel points to it.
fn create_channel(context: Context) {
  let request: NewChannel = match serde_json::from_value(context.input().clone()) {
    Ok(request) => request,
    Err(error) => return settle(&context, Err(error.into())),
  };
  let id = match new_id(context.as_ref(), "channel", &request.name) {
    Ok(id) => id,
    Err(error) => return settle(&context, Err(error)),
  };
  let author = context.actor().identity.clone().unwrap_or_default();
  let origin = json!({"kind": request.kind, "name": request.name, "manage": request.manage.unwrap_or_else(|| vec![author])});
  let reference = NewRef { channel: id.clone(), kind: request.kind, name: request.name, slug: request.slug, cascade: request.cascade };
  let create = json!({"space": id, "package": "channel", "name": "channel"});
  let creating = Rc::clone(&context);
  let reply: Reply = Box::new(move |created| {
    if let Err(error) = created {
      return settle(&creating, Err(error));
    }
    let call = json!({"space": id, "key": "POST [channel]/origin", "input": origin});
    after(&creating, "call", call, move |context, _| add_ref(context.as_ref(), reference).map(|reference| json!(reference)))
  });
  Rc::clone(&context).effect("create", create, reply)
}

// A service in this channel: its space is created, running the package
// registered on this device, and the channel names it.
fn create_service(context: Context) {
  let request: NewService = match serde_json::from_value(context.input().clone()) {
    Ok(request) => request,
    Err(error) => return settle(&context, Err(error.into())),
  };
  let taken = context.get("SELECT 1 AS present FROM service WHERE name = ?", &[json!(request.name)]).map(|row| !row.is_null());
  if let Err(error) = taken.and_then(|taken| if taken || request.name == "channel" { Err(conflict(format!("This channel already names a service [{}]", request.name))) } else { Ok(()) }) {
    return settle(&context, Err(error));
  }
  let id = match new_id(context.as_ref(), "service", &request.name) {
    Ok(id) => id,
    Err(error) => return settle(&context, Err(error)),
  };
  let package = request.package.unwrap_or_else(|| request.name.clone());
  let create = json!({"space": id, "package": package, "name": request.name});
  after(&context, "create", create, move |context, created| {
    let named = Named { name: request.name, service: id, package, version: created["version"].as_str().unwrap_or_default().to_string() };
    context.run("INSERT INTO service (name, space, package, version) VALUES (?, ?, ?, ?)", &[json!(named.name), json!(named.service), json!(named.package), json!(named.version)])?;
    Ok(json!({"name": named.name, "service": named.service, "version": named.version}))
  })
}

fn services(db: &dyn Db) -> Result<Vec<Value>> {
  db.all("SELECT name, space AS service, package, version FROM service ORDER BY name", &[])
}

// An invitation: p2panda takes an invitation key as a member, and the code
// carries its secret to the contact.
fn invite(context: Context) {
  let input = context.input().clone();
  let by = context.actor().identity.clone().unwrap_or_default();
  after(&context, "invite", input.clone(), move |context, invited| {
    context.run(
      "INSERT INTO invitation (invitation, contact, access, invited_by) VALUES (?, ?, ?, ?)",
      &[invited["invitation"].clone(), input["contact"].clone(), input["access"].clone(), json!(by)],
    )?;
    Ok(invited)
  })
}

fn invitations(db: &dyn Db) -> Result<Value> {
  Ok(json!(db.all("SELECT invitation, contact, access, invited_by AS by FROM invitation ORDER BY invitation", &[])?))
}

fn cancel(context: Context, invitation: String) {
  after(&context, "uninvite", json!({"invitation": invitation}), move |context, _| {
    context.run("DELETE FROM invitation WHERE invitation = ?", &[json!(invitation)])?;
    Ok(json!({}))
  })
}

fn members(db: &dyn Db) -> Result<Value> {
  Ok(json!(db.all("SELECT identity, access FROM member ORDER BY identity", &[])?))
}

fn remove(context: Context, identity: String) {
  after(&context, "remove", json!({"identity": identity}), move |context, _| {
    context.run("DELETE FROM member WHERE identity = ?", &[json!(identity)])?;
    Ok(json!({}))
  })
}

// Join the channel an invitation's code names, and point to it from this one.
fn join(context: Context) {
  after(&context, "join", context.input().clone(), |context, joined| {
    let reference = NewRef {
      channel: field(&joined, "channel")?,
      kind: field(&joined, "kind")?,
      name: field(&joined, "name")?,
      slug: None,
      cascade: vec![],
    };
    add_ref(context.as_ref(), reference).map(|reference| json!(reference))
  })
}

fn field(value: &Value, name: &str) -> Result<String> {
  value[name].as_str().map(str::to_string).ok_or_else(|| ServiceError::new(502, "BAD_EFFECT", format!("An effect answered without '{name}'")))
}

fn leave(context: Context) {
  after(&context, "leave", Value::Null, |_, _| Ok(json!({})))
}

fn change_manage(context: Context) {
  #[derive(Deserialize)]
  #[serde(deny_unknown_fields)]
  struct Add {
    add: Vec<String>,
  }
  let Add { add } = match serde_json::from_value(context.input().clone()) {
    Ok(add) => add,
    Err(error) => return settle(&context, Err(error.into())),
  };
  after(&context, "manage", json!({"add": add}), move |context, _| {
    let mut manage = origin(context.as_ref())?.manage;
    manage.extend(add.into_iter().filter(|identity| !manage.contains(identity)).collect::<Vec<_>>());
    context.run("UPDATE origin SET manage = ?", &[json!(manage)])?;
    Ok(json!(manage))
  })
}

// A second ref to the same channel replaces the first.
fn add_ref(db: &dyn Db, new: NewRef) -> Result<Ref> {
  let base = new.slug.clone().unwrap_or_else(|| slugify(&new.name));
  if base.is_empty() || slugify(&base) != base {
    return Err(bad_request(format!("'{}' gives no usable slug", new.name)));
  }
  let taken = |slug: &str| find_ref(db, &new.kind, slug).ok().flatten().is_some_and(|found| found.channel != new.channel);
  let reference = Ref { slug: unique_slug(&base, taken), channel: new.channel, kind: new.kind, name: new.name, cascade: new.cascade };
  db.run(
    "INSERT INTO ref (channel, kind, name, slug, cascade) VALUES (?, ?, ?, ?, ?)
       ON CONFLICT (channel) DO UPDATE SET kind = excluded.kind, name = excluded.name, slug = excluded.slug, cascade = excluded.cascade",
    &[json!(reference.channel), json!(reference.kind), json!(reference.name), json!(reference.slug), json!(reference.cascade)],
  )?;
  Ok(reference)
}

fn to_ref(row: &Value) -> Result<Ref> {
  Ok(Ref {
    channel: row["channel"].as_str().unwrap_or_default().to_string(),
    kind: row["kind"].as_str().unwrap_or_default().to_string(),
    name: row["name"].as_str().unwrap_or_default().to_string(),
    slug: row["slug"].as_str().unwrap_or_default().to_string(),
    cascade: json_column(row, "cascade")?,
  })
}

fn refs(db: &dyn Db) -> Result<Vec<Ref>> {
  db.all("SELECT channel, kind, name, slug, cascade FROM ref ORDER BY kind, slug", &[])?.iter().map(to_ref).collect()
}

fn find_ref(db: &dyn Db, kind: &str, slug: &str) -> Result<Option<Ref>> {
  let row = db.get("SELECT channel, kind, name, slug, cascade FROM ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)])?;
  if row.is_null() { Ok(None) } else { to_ref(&row).map(Some) }
}
