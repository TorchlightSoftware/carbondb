//! `[probe]`: a test fixture that reports what a service sees of its host,
//! built as a component for the core's tests.
//!
//!   POST|GET /seen   the clock, randomness and op ids it sees, and with
//!                    `inner`, what a call inside the op saw too
//!   POST /note       insert a note, keyed by the op
//!   GET /notes       the notes
//!   POST /trap       panic, trapping the instance
//!   POST /call       make the call in its input, answering what it answered
//!   POST|GET /fetch  send every request in `requests` ([{url, method, body}],
//!                    each url http:// or https://)
//!                    over wasi:http at once, writing each body after sending
//!                    it, then answer each response ({status, body}) or the
//!                    failure it met ({error}), in order; a write first reads
//!                    its notes, then notes what came back

use std::rc::Rc;

use carbondb_service_bindings::error::{Result, unknown};
use carbondb_service_bindings::{Db, HandlerContext, Package};
use serde_json::{Value, json};

pub const PACKAGE: Package = Package { setup, entry };

#[cfg(target_arch = "wasm32")]
carbondb_service_bindings::serve!(PACKAGE);

fn setup(db: &dyn Db) -> Result<()> {
  db.run("CREATE TABLE IF NOT EXISTS note (id TEXT PRIMARY KEY, text TEXT NOT NULL) WITHOUT ROWID", &[]).map(|_| ())
}

fn entry(context: Rc<dyn HandlerContext>) {
  match (context.method(), context.resource()) {
    (_, "/seen") => seen(context),
    ("POST", "/note") => {
      let id = context.op().map(|op| op.id());
      let result = context.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(id), context.input()["text"].clone()]).map(|_| json!({"id": id}));
      let _ = context.settle(result);
    }
    ("GET", "/notes") => {
      let result = context.all("SELECT text FROM note ORDER BY text", &[]).map(Value::from);
      let _ = context.settle(result);
    }
    ("POST", "/trap") => panic!("the probe traps"),
    (_, "/fetch") => {
      let result = fetched(context.as_ref());
      let _ = context.settle(result);
    }
    ("POST", "/call") => {
      let (key, input) = (context.input()["key"].as_str().unwrap_or_default().to_string(), context.input()["input"].clone());
      let caller = Rc::clone(&context);
      context.call(&key, input, Box::new(move |result| {
        let _ = caller.settle(result);
      }));
    }
    (method, resource) => {
      let _ = context.settle(Err(unknown(method, resource)));
    }
  }
}

// What it sees: the wall clock, random bytes, and two ids drawn from the op
// around a call inside it, when the input asks for one.
fn seen(context: Rc<dyn HandlerContext>) {
  let first = context.op().map(|op| op.id());
  let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or_default();
  let seen = json!({"clock": clock, "random": random(), "first": first});
  if context.input()["inner"] != json!(true) {
    let _ = context.settle(Ok(seen));
    return;
  }
  let (caller, key) = (Rc::clone(&context), format!("{} [probe]/seen", context.method()));
  context.call(&key, json!({}), Box::new(move |inner| {
    let last = caller.op().map(|op| op.id());
    let _ = caller.settle(inner.map(|inner| json!({"outer": seen, "inner": inner, "last": last})));
  }));
}

// Send the requests in the input, and note in a write what came back.
fn fetched(context: &dyn HandlerContext) -> Result<Value> {
  let writes = context.op().is_some();
  if writes {
    context.all("SELECT text FROM note", &[])?;
  }
  let requests = context.input()["requests"].as_array().cloned().unwrap_or_default();
  let responses = Value::from(outside::fetch_all(&requests));
  if writes {
    let id = context.op().map(|op| op.id());
    context.run("INSERT INTO note (id, text) VALUES (?, ?)", &[json!(id), json!(responses.to_string())])?;
  }
  Ok(responses)
}

fn random() -> Vec<u8> {
  let mut bytes = vec![0u8; 8];
  fill(&mut bytes);
  bytes
}

#[cfg(target_arch = "wasm32")]
fn fill(bytes: &mut [u8]) {
  getrandom::fill(bytes).expect("WASI gives randomness");
}

// Natively, the probe is never run.
#[cfg(not(target_arch = "wasm32"))]
fn fill(_: &mut [u8]) {}

// HTTP over wasi:http, as any Rust library on it sends it: every request
// sent before any response is awaited.
#[cfg(target_arch = "wasm32")]
mod outside {
  use serde_json::{Value, json};
  use wasip2::http::outgoing_handler::{self, FutureIncomingResponse};
  use wasip2::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, Scheme};
  use wasip2::io::streams::StreamError;

  pub fn fetch_all(requests: &[Value]) -> Vec<Value> {
    let sent: Vec<Result<FutureIncomingResponse, String>> = requests.iter().map(send).collect();
    sent.into_iter().map(|sent| sent.and_then(received).unwrap_or_else(|error| json!({"error": error}))).collect()
  }

  // Send a request, then write its body.
  fn send(request: &Value) -> Result<FutureIncomingResponse, String> {
    let url = request["url"].as_str().unwrap_or_default();
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (authority, path) = rest.split_once('/').map(|(authority, path)| (authority, format!("/{path}"))).unwrap_or((rest, "/".into()));
    let outgoing = OutgoingRequest::new(Fields::new());
    let method = if request["method"] == "POST" { Method::Post } else { Method::Get };
    outgoing.set_method(&method).map_err(|_| "method")?;
    outgoing.set_scheme(Some(if scheme == "https" { &Scheme::Https } else { &Scheme::Http })).map_err(|_| "scheme")?;
    outgoing.set_authority(Some(authority)).map_err(|_| "authority")?;
    outgoing.set_path_with_query(Some(&path)).map_err(|_| "path")?;
    let body = outgoing.body().map_err(|_| "body")?;
    let future = outgoing_handler::handle(outgoing, None).map_err(|error| format!("{error:?}"))?;
    let stream = body.write().map_err(|_| "write")?;
    stream.blocking_write_and_flush(request["body"].as_str().unwrap_or_default().as_bytes()).map_err(|error| format!("{error:?}"))?;
    drop(stream);
    OutgoingBody::finish(body, None).map_err(|error| format!("{error:?}"))?;
    Ok(future)
  }

  // A response, read whole, or the failure it met.
  fn received(future: FutureIncomingResponse) -> Result<Value, String> {
    future.subscribe().block();
    let response = future.get().ok_or("no response")?.map_err(|_| "taken twice")?.map_err(|error| format!("{error:?}"))?;
    let status = response.status();
    let incoming = response.consume().map_err(|_| "consume")?;
    let stream = incoming.stream().map_err(|_| "stream")?;
    let mut body = vec![];
    loop {
      match stream.blocking_read(65536) {
        Ok(chunk) => body.extend(chunk),
        Err(StreamError::Closed) => break,
        Err(error) => return Err(format!("{error:?}")),
      }
    }
    Ok(json!({"status": status, "body": String::from_utf8_lossy(&body)}))
  }
}

// Natively, the probe is never run.
#[cfg(not(target_arch = "wasm32"))]
mod outside {
  pub fn fetch_all(_: &[serde_json::Value]) -> Vec<serde_json::Value> {
    vec![]
  }
}
