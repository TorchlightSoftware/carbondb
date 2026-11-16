//! `wasi:http`, as a component reaches the outside world: every outgoing
//! request is the core's `fetch` effect (crate::ops::fetch), so it is made
//! once in a write and recorded for every replay, and made every time in a
//! GET, whatever HTTP library the service uses.
//!
//! Wasmtime hands the host a request when the instance sends it, on the
//! core's thread, and takes back a response it may finish on another. So the
//! request takes its slot in the op's effects right then, in the order the
//! service sent it; its body, which the instance may still be writing, is
//! read wherever the response is awaited; and the whole request crosses back
//! to the core's thread, which makes it as the effect and answers.

use std::future::Future;
use std::rc::Rc;

use http_body_util::{BodyExt, Full};
use serde_json::Value;
use tokio::sync::oneshot;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

use crate::error::{Result, internal};
use crate::ops::fetch::{Request, Response};
use crate::calls::service_call::{Asked, ServiceCall};

// What an instance sends its requests through: the core, for what it runs for.
pub(super) struct Outside {
  pub(super) running: u64,
}

// A response, and the work that carries its body, as wasmtime takes them back.
type Answer = Box<dyn Future<Output = std::result::Result<(http::Response<WasiBody>, Carrying), Error>> + Send>;
type Carrying = Box<dyn Future<Output = std::result::Result<(), Error>> + Send>;

impl WasiHttpHooks for Outside {
  fn send_request(&mut self, request: http::Request<WasiBody>, _options: Option<RequestOptions>, _sent: Carrying) -> Answer {
    let (head, body) = request.into_parts();
    let fetching = super::host::call(self.running).and_then(|call| ask(call, &head));
    Box::new(async move {
      let body = body.collect().await?.to_bytes();
      let output = match fetching {
        Ok(fetching) => fetching.answer(Request::of(&head, &body)).await,
        Err(error) => Err(error),
      };
      Ok((response(output)?, Box::new(async { Ok(()) }) as Carrying))
    })
  }
}

// A request asked for: its recorded response, or the core making it once its body is read.
enum Fetching {
  Recorded(Result<Value>),
  Making { request: oneshot::Sender<Value>, response: oneshot::Receiver<Result<Value>> },
}

// Ask for a request as the call's `fetch` effect, on the core's thread, now:
// it takes its slot, or gets its recorded response. Making it waits for its
// body, then runs on the core's thread as a task.
fn ask(call: Rc<ServiceCall>, head: &http::request::Parts) -> Result<Fetching> {
  let asked = serde_json::to_value(Request::of(head, &[])).expect("a request is JSON");
  let slot = match call.ask_effect("fetch", &asked)? {
    Asked::Make(slot) => slot,
    Asked::Recorded(output) => return Ok(Fetching::Recorded(output)),
  };
  let (request, requested) = oneshot::channel::<Value>();
  let (answer, response) = oneshot::channel();
  let carbon = Rc::clone(call.carbon());
  let making = async move {
    if let Ok(input) = requested.await {
      call.make_effect(slot, "fetch", input, Box::new(move |output| {
        let _ = answer.send(output);
      }));
    }
  };
  let _ = carbon.spawn(Box::pin(making));
  Ok(Fetching::Making { request, response })
}

impl Fetching {
  // The response to the whole request.
  async fn answer(self, request: Request) -> Result<Value> {
    match self {
      Fetching::Recorded(output) => output,
      Fetching::Making { request: making, response } => {
        let _ = making.send(serde_json::to_value(request).expect("a request is JSON"));
        response.await.unwrap_or_else(|_| Err(internal("The call ended before its request was made")))
      }
    }
  }
}

// What the instance receives: the response, or the failure the request met,
// which a replay asking for more than its first run made meets as unreachable.
fn response(output: Result<Value>) -> std::result::Result<http::Response<WasiBody>, Error> {
  let response = output.and_then(|value| serde_json::from_value::<Response>(value).map_err(|error| internal(format!("A malformed response: {error}"))).and_then(Response::into_http));
  match response {
    Ok(response) => Ok(response.map(|body| Full::new(body).map_err(|never: std::convert::Infallible| match never {}).boxed_unsync())),
    Err(error) => Err(Error::InternalError(Some(error.message))),
  }
}
