//! `[space]`: a space's origin, its refs to other spaces, and later its members
//! and manage list. It runs in every replicated space. Every POST but the
//! origin needs manage on the space.

pub mod names;

use std::rc::Rc;

use carbondb_service_bindings::error::{Result, bad_request, conflict, forbidden, not_built, not_found, single_node, unknown};
use carbondb_service_bindings::{Db, HandlerContext, Package, migrations};
use names::{slugify, unique_slug};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

include!(concat!(env!("OUT_DIR"), "/package.rs"));

pub const PACKAGE: Package = Package { name: NAME, version: VERSION, binding: BINDING, init, entry };

fn init(db: &dyn Db) -> Result<()> {
  migrations::apply(db, MIGRATIONS)
}

// What a space is, fixed by its first op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
  pub kind: String,
  pub name: String,
  pub services: Vec<String>,
  pub manage: Vec<String>,
}

// A pointer from this space to another. It grants nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ref {
  pub space: String,
  pub kind: String,
  pub name: String,
  pub slug: String,
  #[serde(default)]
  pub cascade: Vec<String>,
}

// A ref as a call asks for it: its slug defaults to the name's, and is made
// unique among this space's refs of the same kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRef {
  space: String,
  kind: String,
  name: String,
  slug: Option<String>,
  #[serde(default)]
  cascade: Vec<String>,
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
  let input = || context.input().clone();
  match (context.method(), context.resource()) {
    ("POST", "/origin") => settle(&context, serde_json::from_value(input()).map_err(Into::into).and_then(|origin| write_origin(context.as_ref(), origin))),
    ("GET", "/origin") => settle(&context, origin(context.as_ref()).map(|origin| json!(origin))),
    ("GET", "/manage") => {
      let caller = Rc::clone(&context);
      managers(&context, Box::new(move |managers| settle(&caller, managers.map(|managers| json!(managers)))))
    }
    ("POST", "/ref") => {
      let caller = Rc::clone(&context);
      require_manage(&context, Box::new(move |allowed| {
        let added = allowed.and_then(|_| Ok(serde_json::from_value(caller.input().clone())?)).and_then(|new| add_ref(caller.as_ref(), new));
        settle(&caller, added.map(|reference| json!(reference)))
      }))
    }
    ("GET", "/refs") => settle(&context, refs(context.as_ref()).map(|refs| json!(refs))),
    ("GET", "/ref") => {
      let found = serde_json::from_value(input()).map_err(Into::into).and_then(|RefName { kind, slug }| find_ref(context.as_ref(), &kind, &slug));
      settle(&context, found.map(|reference| json!(reference)))
    }
    ("POST", "/members") => settle(&context, Err(single_node("Adding a member"))),
    ("POST", "/manage") => settle(&context, Err(not_built("Changing a manage list"))),
    (method, resource) => settle(&context, Err(unknown(method, resource))),
  }
}

fn settle(context: &Context, result: Result<Value>) {
  let _ = context.settle(result);
}

fn json_column(row: &Value, column: &str) -> Result<Vec<String>> {
  Ok(serde_json::from_str(row[column].as_str().unwrap_or("[]"))?)
}

fn write_origin(db: &dyn Db, origin: Origin) -> Result<Value> {
  if !db.get("SELECT 1 AS present FROM origin", &[])?.is_null() {
    return Err(conflict("A space's origin is written once"));
  }
  db.run(
    "INSERT INTO origin (one, kind, name, services, manage) VALUES (1, ?, ?, ?, ?)",
    &[json!(origin.kind), json!(origin.name), json!(origin.services), json!(origin.manage)],
  )?;
  Ok(json!(origin))
}

fn origin(db: &dyn Db) -> Result<Origin> {
  let row = db.get("SELECT kind, name, services, manage FROM origin", &[])?;
  if row.is_null() {
    return Err(not_found("This space has no origin yet"));
  }
  Ok(Origin {
    kind: row["kind"].as_str().unwrap_or_default().to_string(),
    name: row["name"].as_str().unwrap_or_default().to_string(),
    services: json_column(&row, "services")?,
    manage: json_column(&row, "manage")?,
  })
}

type Answer<T> = Box<dyn FnOnce(Result<T>)>;

// The identities that manage this space, with `self` resolved by `[identity]`
// to the identity whose identity space this is. Only an identity space names `self`.
fn managers(context: &Context, answer: Answer<Vec<String>>) {
  let manage = match origin(context.as_ref()) {
    Ok(origin) => origin.manage,
    Err(error) => return answer(Err(error)),
  };
  if !manage.iter().any(|entry| entry == "self") {
    return answer(Ok(manage));
  }
  Rc::clone(context).call("GET [identity]/origin", Value::Null, Box::new(move |identity| {
    answer(identity.map(|identity| resolve_self(manage, identity["id"].as_str())))
  }))
}

fn resolve_self(manage: Vec<String>, id: Option<&str>) -> Vec<String> {
  manage.into_iter().filter_map(|entry| if entry == "self" { id.map(str::to_string) } else { Some(entry) }).collect()
}

fn require_manage(context: &Context, answer: Answer<()>) {
  let Some(actor) = context.actor().identity.clone() else {
    return answer(Err(forbidden("Managing a space needs an acting identity")));
  };
  managers(context, Box::new(move |managers| {
    answer(managers.and_then(|managers| if managers.contains(&actor) { Ok(()) } else { Err(forbidden("Only a manager of this space may change it")) }))
  }))
}

// A second ref to the same space replaces the first.
fn add_ref(db: &dyn Db, new: NewRef) -> Result<Ref> {
  let base = new.slug.clone().unwrap_or_else(|| slugify(&new.name));
  if base.is_empty() || slugify(&base) != base {
    return Err(bad_request(format!("'{}' gives no usable slug", new.name)));
  }
  let taken = |slug: &str| find_ref(db, &new.kind, slug).ok().flatten().is_some_and(|found| found.space != new.space);
  let reference = Ref { slug: unique_slug(&base, taken), space: new.space, kind: new.kind, name: new.name, cascade: new.cascade };
  db.run(
    "INSERT INTO ref (space, kind, name, slug, cascade) VALUES (?, ?, ?, ?, ?)
       ON CONFLICT (space) DO UPDATE SET kind = excluded.kind, name = excluded.name, slug = excluded.slug, cascade = excluded.cascade",
    &[json!(reference.space), json!(reference.kind), json!(reference.name), json!(reference.slug), json!(reference.cascade)],
  )?;
  Ok(reference)
}

fn to_ref(row: &Value) -> Result<Ref> {
  Ok(Ref {
    space: row["space"].as_str().unwrap_or_default().to_string(),
    kind: row["kind"].as_str().unwrap_or_default().to_string(),
    name: row["name"].as_str().unwrap_or_default().to_string(),
    slug: row["slug"].as_str().unwrap_or_default().to_string(),
    cascade: json_column(row, "cascade")?,
  })
}

fn refs(db: &dyn Db) -> Result<Vec<Ref>> {
  db.all("SELECT space, kind, name, slug, cascade FROM ref ORDER BY kind, slug", &[])?.iter().map(to_ref).collect()
}

fn find_ref(db: &dyn Db, kind: &str, slug: &str) -> Result<Option<Ref>> {
  let row = db.get("SELECT space, kind, name, slug, cascade FROM ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)])?;
  if row.is_null() { Ok(None) } else { to_ref(&row).map(Some) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resolves_self_to_the_identity_or_drops_it() {
    let manage = || vec!["self".to_string(), "bob".to_string()];
    assert_eq!(resolve_self(manage(), Some("alice")), ["alice", "bob"]);
    assert_eq!(resolve_self(manage(), None), ["bob"]);
  }
}
