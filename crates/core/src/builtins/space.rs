//! `[space]`: a space's origin, its refs to other spaces, and later its members
//! and manage list. It runs in every replicated space. Every POST but the
//! origin needs manage on the space.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::builtins::identity;
use crate::error::{Result, bad_request, conflict, forbidden, not_built, not_found, single_node};
use crate::invocation::Invocation;
use crate::names::{slugify, unique_slug};
use crate::store::Db;

pub const MIGRATIONS: &[&str] = &[include_str!("../../sql/builtins/space/001.sql")];

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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRef {
  space: String,
  kind: String,
  name: String,
  slug: Option<String>,
  #[serde(default)]
  cascade: Vec<String>,
}

pub fn handle(invocation: &Invocation) -> Result<Value> {
  let input = || invocation.input.clone();
  match (invocation.call.method(), invocation.call.resource.as_str()) {
    ("POST", "/origin") => {
      let origin: Origin = serde_json::from_value(input())?;
      write_origin(invocation, &origin)?;
      Ok(json!(origin))
    }
    ("POST", "/ref") => {
      require_manage(invocation, invocation.actor.identity.as_deref())?;
      Ok(json!(add_ref(invocation, serde_json::from_value(input())?)?))
    }
    ("GET", "/refs") => Ok(json!(refs(invocation)?)),
    ("POST", "/members") => Err(single_node("Adding a member")),
    ("POST", "/manage") => Err(not_built("Changing a manage list")),
    _ => Err(invocation.call.unknown()),
  }
}

pub(crate) fn json_column(row: &Value, column: &str) -> Result<Vec<String>> {
  Ok(serde_json::from_str(row[column].as_str().unwrap_or("[]"))?)
}

fn write_origin(db: &dyn Db, origin: &Origin) -> Result<()> {
  if !db.get("SELECT 1 AS present FROM carbon_origin", &[])?.is_null() {
    return Err(conflict("A space's origin is written once"));
  }
  db.run(
    "INSERT INTO carbon_origin (one, kind, name, services, manage) VALUES (1, ?, ?, ?, ?)",
    &[json!(origin.kind), json!(origin.name), json!(origin.services), json!(origin.manage)],
  )?;
  Ok(())
}

pub fn origin(db: &dyn Db) -> Result<Origin> {
  let row = db.get("SELECT kind, name, services, manage FROM carbon_origin", &[])?;
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

// The identities that manage this space, with `self` resolved to the identity
// whose identity space this is. Only an identity space names `self`.
pub fn managers(db: &dyn Db) -> Result<Vec<String>> {
  let resolve = |entry: String| if entry == "self" { identity::id(db) } else { Ok(Some(entry)) };
  let resolved: Vec<Option<String>> = origin(db)?.manage.into_iter().map(resolve).collect::<Result<_>>()?;
  Ok(resolved.into_iter().flatten().collect())
}

pub fn require_manage(db: &dyn Db, actor: Option<&str>) -> Result<()> {
  let actor = actor.ok_or_else(|| forbidden("Managing a space needs an acting identity"))?;
  if managers(db)?.iter().any(|manager| manager == actor) {
    Ok(())
  } else {
    Err(forbidden("Only a manager of this space may change it"))
  }
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
    "INSERT INTO carbon_ref (space, kind, name, slug, cascade) VALUES (?, ?, ?, ?, ?)
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

pub fn refs(db: &dyn Db) -> Result<Vec<Ref>> {
  db.all("SELECT space, kind, name, slug, cascade FROM carbon_ref ORDER BY kind, slug", &[])?.iter().map(to_ref).collect()
}

pub fn find_ref(db: &dyn Db, kind: &str, slug: &str) -> Result<Option<Ref>> {
  let row = db.get("SELECT space, kind, name, slug, cascade FROM carbon_ref WHERE kind = ? AND slug = ?", &[json!(kind), json!(slug)])?;
  if row.is_null() { Ok(None) } else { to_ref(&row).map(Some) }
}
