//! The `fetch` effect: an HTTP request to the outside world, and its whole
//! response. Every outgoing request a service makes becomes this effect,
//! whatever language it was written in: a component reaches the network only
//! through `wasi:http`, and the WASM host hands each request it is given to
//! the core as `fetch` (crate::wasm). Like any effect, in a write it is made
//! once, on the op's first run, and its response is recorded on the op's row
//! for every replay; in a GET it is made every time.
//!
//! A request and a response cross as JSON, bodies in base64, since an op's row
//! records them: `{method, url, headers, body}` and `{status, headers, body}`,
//! headers as `[name, value]` pairs in order. A response is read in full
//! before it is answered, so the row records all of it.

use std::rc::Rc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::error::{Result, ServiceError, bad_request};
use crate::calls::service_call::ServiceCall;
use crate::Reply;

// An outgoing request, as an op's row records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
  pub method: String,
  pub url: String,
  pub headers: Vec<(String, String)>,
  #[serde(with = "base64_body")]
  pub body: Vec<u8>,
}

// Its response, read in full, as an op's row records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
  pub status: u16,
  pub headers: Vec<(String, String)>,
  #[serde(with = "base64_body")]
  pub body: Vec<u8>,
}

impl Request {
  // A request from its head and its body.
  pub fn of(head: &http::request::Parts, body: &[u8]) -> Request {
    Request { method: head.method.to_string(), url: head.uri.to_string(), headers: pairs(&head.headers), body: body.to_vec() }
  }

  fn into_http(self) -> Result<http::Request<Full<Bytes>>> {
    let mut built = http::Request::builder().method(self.method.as_str()).uri(self.url.as_str());
    for (name, value) in &self.headers {
      built = built.header(name, value);
    }
    built.body(Full::new(Bytes::from(self.body))).map_err(|error| bad_request(format!("Not a request this device can make: {error}")))
  }
}

impl Response {
  // A response from its head and its whole body.
  fn of(head: &http::response::Parts, body: &[u8]) -> Response {
    Response { status: head.status.as_u16(), headers: pairs(&head.headers), body: body.to_vec() }
  }

  // The response as HTTP again, with its body whole.
  pub fn into_http(self) -> Result<http::Response<Bytes>> {
    let mut built = http::Response::builder().status(self.status);
    for (name, value) in &self.headers {
      built = built.header(name, value);
    }
    built.body(Bytes::from(self.body)).map_err(|error| bad_request(format!("A malformed recorded response: {error}")))
  }
}

fn pairs(headers: &http::HeaderMap) -> Vec<(String, String)> {
  headers.iter().map(|(name, value)| (name.to_string(), String::from_utf8_lossy(value.as_bytes()).into_owned())).collect()
}

// Make the request in the input, answering its whole response. The request
// runs wherever the host's network runs; its answer comes back to the core's
// thread as a task there.
pub(crate) fn fetch_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
  let request = serde_json::from_value::<Request>(input).map_err(|error| bad_request(format!("A fetch is {{method, url, headers, body}}: {error}"))).and_then(Request::into_http);
  let request = match request {
    Ok(request) => request,
    Err(error) => return reply(Err(error)),
  };
  let sending = wasmtime_wasi::runtime::spawn(send(request));
  let answering = async move { reply(sending.await) };
  let _ = caller.carbon().spawn(Box::pin(answering));
}

// Send a request and read its whole response.
async fn send(request: http::Request<Full<Bytes>>) -> Result<Value> {
  let request = request.map(|body| body.map_err(|never| match never {}));
  let (response, connection) = wasmtime_wasi_http::default_send_request(request, None).await.map_err(failed)?;
  // The connection runs beside the body it carries, until the body is read.
  let _connection = wasmtime_wasi::runtime::spawn(connection);
  let (head, body) = response.into_parts();
  let body = body.collect().await.map_err(failed)?.to_bytes();
  Ok(serde_json::to_value(Response::of(&head, &body)).expect("a response is JSON"))
}

// A request that failed to reach its server or to come back whole.
fn failed(error: wasmtime_wasi_http::Error) -> ServiceError {
  ServiceError::new(502, "FETCH_FAILED", error.to_string())
}

mod base64_body {
  use super::*;

  pub fn serialize<S: Serializer>(body: &[u8], serializer: S) -> std::result::Result<S::Ok, S::Error> {
    STANDARD.encode(body).serialize(serializer)
  }

  pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error> {
    STANDARD.decode(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_request_and_a_response_cross_as_json_with_their_bodies_in_base64() {
    let request = Request { method: "POST".into(), url: "http://example.com/a?b=c".into(), headers: vec![("x-a".into(), "1".into()), ("x-a".into(), "2".into())], body: b"\x00hi".to_vec() };
    let crossed = serde_json::to_value(&request).unwrap();
    assert_eq!(crossed, json!({"method": "POST", "url": "http://example.com/a?b=c", "headers": [["x-a", "1"], ["x-a", "2"]], "body": "AGhp"}));
    assert_eq!(serde_json::from_value::<Request>(crossed).unwrap(), request);
    let response = Response { status: 201, headers: vec![("content-type".into(), "text/plain".into())], body: b"made".to_vec() };
    let http = response.clone().into_http().unwrap();
    assert_eq!((http.status().as_u16(), http.headers()["content-type"].to_str().unwrap(), http.body().as_ref()), (201, "text/plain", &b"made"[..]));
  }
}
