//! A Rust service as a WebAssembly component, behind the `guest` feature: the
//! same service, written against `HandlerContext` and `Db`, exported through
//! the `carbondb:service` world (`wit/service.wit`). A service crate built for
//! `wasm32-wasip2` exports itself with `serve!(PACKAGE)`.
//!
//! Each call reaches the component as `handle`, with its request as JSON; the
//! service sees it as a handler context whose queries, draws, calls and
//! effects go to the core through the world's `host` imports. `handle`
//! answers once the service settles the context.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Result, ServiceError, conflict};
use crate::{Actor, Db, HandlerContext, Op, Package, Reply};

pub mod bindings {
  wit_bindgen::generate!({
    path: "wit",
    world: "service",
    pub_export_macro: true,
    export_macro_name: "export_service",
    default_bindings_module: "carbondb_service_bindings::guest::bindings",
    // The world's WASI 0.2 is the `wasip2` crate's, shared with whatever HTTP
    // library the service uses; its WASI 0.3, which Rust's libraries hardly
    // use yet, is generated here.
    with: {
      "wasi:clocks/types@0.3.0": generate,
      "wasi:http/types@0.3.0": generate,
      "wasi:http/client@0.3.0": generate,
      "wasi:http/outgoing-handler@0.2.12": wasip2::http::outgoing_handler,
      "wasi:http/types@0.2.12": wasip2::http::types,
      "wasi:io/error@0.2.12": wasip2::io::error,
      "wasi:io/poll@0.2.12": wasip2::io::poll,
      "wasi:io/streams@0.2.12": wasip2::io::streams,
      "wasi:clocks/monotonic-clock@0.2.12": wasip2::clocks::monotonic_clock,
    },
  });
}

use bindings::carbondb::service::host;

// Export a package as this component's service.
#[macro_export]
macro_rules! serve {
  ($package:expr) => {
    struct CarbondbService;

    impl $crate::guest::bindings::Guest for CarbondbService {
      fn setup() -> ::core::result::Result<(), ::std::string::String> {
        $crate::guest::setup(&$package)
      }

      async fn handle(request: ::std::string::String) -> ::core::result::Result<::std::string::String, ::std::string::String> {
        $crate::guest::handle(&$package, request).await
      }
    }

    $crate::guest::bindings::export_service!(CarbondbService with_types_in $crate::guest::bindings);
  };
}

// Set the service's schema up on its binding.
pub fn setup(package: &Package) -> std::result::Result<(), String> {
  (package.setup)(&HostDb).map_err(|error| error.to_text())
}

// Handle one call: hand it to the service's entry point, and answer once it settles.
pub async fn handle(package: &Package, request: String) -> std::result::Result<String, String> {
  let call = GuestCall::of(&request).map_err(|error| error.to_text())?;
  let settled = Rc::clone(&call.settled);
  (package.entry)(call);
  Settling(settled).await.map(|value| value.to_string()).map_err(|error| error.to_text())
}

// The `sql` binding, through the core.
struct HostDb;

impl Db for HostDb {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    host::run(sql, &Value::from(params.to_vec()).to_string()).map_err(|text| ServiceError::from_text(&text))
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    let rows = host::all(sql, &Value::from(params.to_vec()).to_string()).map_err(|text| ServiceError::from_text(&text))?;
    serde_json::from_str(&rows).map_err(|error| ServiceError::from_text(&error.to_string()))
  }
}

// One call as the core sent it.
#[derive(Deserialize)]
struct Request {
  method: String,
  resource: String,
  input: Value,
  actor: Actor,
  op: Option<Op>,
}

// A call's result, once the service settles it, and who waits for it.
#[derive(Default)]
struct Settled {
  result: RefCell<Option<Result<Value>>>,
  waiting: RefCell<Option<Waker>>,
}

// The call as the service sees it.
struct GuestCall {
  request: Request,
  settled: Rc<Settled>,
}

impl GuestCall {
  fn of(text: &str) -> Result<Rc<GuestCall>> {
    let mut request: Request = serde_json::from_str(text).map_err(|error| ServiceError::from_text(&error.to_string()))?;
    request.op = request.op.map(|op| op.drawn_by(|| host::draw().unwrap_or_else(|text| panic!("{}", ServiceError::from_text(&text)))));
    Ok(Rc::new(GuestCall { request, settled: Rc::default() }))
  }
}

impl Db for GuestCall {
  fn run(&self, sql: &str, params: &[Value]) -> Result<u64> {
    HostDb.run(sql, params)
  }

  fn all(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>> {
    HostDb.all(sql, params)
  }
}

impl HandlerContext for GuestCall {
  fn method(&self) -> &str {
    &self.request.method
  }

  fn resource(&self) -> &str {
    &self.request.resource
  }

  fn input(&self) -> &Value {
    &self.request.input
  }

  fn actor(&self) -> &Actor {
    &self.request.actor
  }

  fn op(&self) -> Option<&Op> {
    self.request.op.as_ref()
  }

  fn call(self: Rc<Self>, key: &str, input: Value, reply: Reply) {
    let (key, input) = (key.to_string(), input.to_string());
    wit_bindgen::spawn_local(async move { reply(answered(host::call(key, input).await)) });
  }

  fn effect(self: Rc<Self>, name: &str, input: Value, reply: Reply) {
    let (name, input) = (name.to_string(), input.to_string());
    wit_bindgen::spawn_local(async move { reply(answered(host::effect(name, input).await)) });
  }

  fn settle(&self, result: Result<Value>) -> Result<()> {
    if self.settled.result.borrow().is_some() {
      return Err(conflict(format!("{} {} has already settled", self.request.method, self.request.resource)));
    }
    self.settled.result.replace(Some(result));
    if let Some(waker) = self.settled.waiting.take() {
      waker.wake();
    }
    Ok(())
  }
}

// What the core answered a call or an effect with.
fn answered(answer: std::result::Result<String, String>) -> Result<Value> {
  match answer {
    Ok(text) => serde_json::from_str(&text).map_err(|error| ServiceError::from_text(&error.to_string())),
    Err(text) => Err(ServiceError::from_text(&text)),
  }
}

// Waits for the service to settle its call.
struct Settling(Rc<Settled>);

impl Future for Settling {
  type Output = Result<Value>;

  fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Result<Value>> {
    match self.0.result.take() {
      Some(result) => Poll::Ready(result),
      None => {
        self.0.waiting.replace(Some(context.waker().clone()));
        Poll::Pending
      }
    }
  }
}
