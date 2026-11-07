//! `[device]`: in an identity space, the device keys it authorizes (origins);
//! in a personal space, each device's name and tags (refs). Writing either
//! needs manage on the space, which `[space]` answers.

use std::rc::Rc;

use carbondb_service_bindings::error::{Result, bad_request, forbidden, single_node, unknown};
use carbondb_service_bindings::{Db, HandlerContext, Package, migrations};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

include!(concat!(env!("OUT_DIR"), "/package.rs"));

pub const PACKAGE: Package = Package { name: NAME, version: VERSION, binding: BINDING, init, entry };

fn init(db: &dyn Db) -> Result<()> {
  migrations::apply(db, "d1_migrations", MIGRATIONS)
}

// A device's private description, kept in the personal space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRef {
  pub pubkey: String,
  pub name: String,
  #[serde(default)]
  pub tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginInput {
  pubkey: String,
}

type Context = Rc<dyn HandlerContext>;

fn entry(context: Context) {
  match (context.method(), context.resource()) {
    ("POST", "/origin") => {
      let caller = Rc::clone(&context);
      require_manage(&context, Box::new(move |allowed| match allowed {
        Ok(()) => in_identity_space(caller),
        Err(error) => settle(&caller, Err(error)),
      }))
    }
    ("GET", "/devices") => settle(&context, devices(context.as_ref()).map(|devices| json!(devices))),
    ("POST", "/ref") => {
      let caller = Rc::clone(&context);
      require_manage(&context, Box::new(move |allowed| {
        let put = allowed.and_then(|_| Ok(serde_json::from_value(caller.input().clone())?)).and_then(|device| put_device_ref(caller.as_ref(), device));
        settle(&caller, put)
      }))
    }
    ("GET", "/refs") => settle(&context, device_refs(context.as_ref()).map(|refs| json!(refs))),
    (method, resource) => settle(&context, Err(unknown(method, resource))),
  }
}

fn settle(context: &Context, result: Result<Value>) {
  let _ = context.settle(result);
}

// Authorize a device key, which only an identity space does.
fn in_identity_space(context: Context) {
  let caller = Rc::clone(&context);
  context.call("GET [space]/origin", Value::Null, Box::new(move |origin| {
    let added = origin.and_then(|origin| {
      if origin["kind"] != json!("identity") {
        return Err(bad_request("A device's origin is written in its identity space"));
      }
      let OriginInput { pubkey } = serde_json::from_value(caller.input().clone())?;
      add_device(caller.as_ref(), &pubkey)
    });
    settle(&caller, added)
  }))
}

fn require_manage(context: &Context, answer: Box<dyn FnOnce(Result<()>)>) {
  let Some(actor) = context.actor().identity.clone() else {
    return answer(Err(forbidden("Managing a space needs an acting identity")));
  };
  Rc::clone(context).call("GET [space]/manage", Value::Null, Box::new(move |managers| {
    answer(managers.and_then(|managers| {
      if managers.as_array().is_some_and(|managers| managers.contains(&json!(actor))) { Ok(()) } else { Err(forbidden("Only a manager of this space may change its devices")) }
    }))
  }))
}

// Authorize the first device's key. A second device is authorized by an
// existing one over the network, which single-node cannot do.
fn add_device(db: &dyn Db, pubkey: &str) -> Result<Value> {
  if !devices(db)?.is_empty() {
    return Err(single_node("Adding a device"));
  }
  db.run("INSERT INTO device (pubkey) VALUES (?)", &[json!(pubkey)])?;
  Ok(json!({"pubkey": pubkey}))
}

fn devices(db: &dyn Db) -> Result<Vec<String>> {
  let rows = db.all("SELECT pubkey FROM device ORDER BY pubkey", &[])?;
  Ok(rows.iter().filter_map(|row| row["pubkey"].as_str().map(str::to_string)).collect())
}

// A second ref for the same device replaces the first.
fn put_device_ref(db: &dyn Db, device: DeviceRef) -> Result<Value> {
  db.run(
    "INSERT INTO device_ref (pubkey, name, tags) VALUES (?, ?, ?)
       ON CONFLICT (pubkey) DO UPDATE SET name = excluded.name, tags = excluded.tags",
    &[json!(device.pubkey), json!(device.name), json!(device.tags)],
  )?;
  Ok(json!(device))
}

fn device_refs(db: &dyn Db) -> Result<Vec<DeviceRef>> {
  db.all("SELECT pubkey, name, tags FROM device_ref ORDER BY name", &[])?
    .iter()
    .map(|row| {
      Ok(DeviceRef {
        pubkey: row["pubkey"].as_str().unwrap_or_default().to_string(),
        name: row["name"].as_str().unwrap_or_default().to_string(),
        tags: serde_json::from_str(row["tags"].as_str().unwrap_or("[]"))?,
      })
    })
    .collect()
}
