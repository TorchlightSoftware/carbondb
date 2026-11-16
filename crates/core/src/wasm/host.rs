//! The world's `host` imports, as the core answers them. Wasmtime asks that
//! what a store holds be thread-safe, and the core runs on one thread, so an
//! instance holds only the id of what it runs for, and reaches it through the
//! table of calls running on the core's thread. Every import runs on that
//! thread, as the instance is polled there.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;
use tokio::sync::oneshot;
use wasmtime::component::{Accessor, ComponentType, Lift, Linker, Lower, ResourceTable};
use wasmtime::{Result, StoreContextMut};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::error::{bad_request, internal};
use crate::calls::service_call::ServiceCall;
use crate::{Db, HandlerContext, Reply, Routing};

// What an instance holds: the id of what it runs for, its closed WASI, and
// its HTTP, which reaches the outside world only through the core.
pub(super) struct Turn {
  running: u64,
  wasi: WasiCtx,
  http: WasiHttpCtx,
  outside: super::http::Outside,
  table: ResourceTable,
}

impl Turn {
  // For what is registered as `running`; `op`, the hash and time of the op
  // a write runs in, seeds its WASI.
  pub(super) fn new(running: u64, op: Option<(&str, u64)>) -> Turn {
    Turn { running, wasi: super::wasi::closed(op), http: WasiHttpCtx::new(), outside: super::http::Outside { running }, table: ResourceTable::new() }
  }
}

impl WasiView for Turn {
  fn ctx(&mut self) -> WasiCtxView<'_> {
    WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
  }
}

impl WasiHttpView for Turn {
  fn http(&mut self) -> WasiHttpCtxView<'_> {
    WasiHttpCtxView { ctx: &mut self.http, table: &mut self.table, hooks: &mut self.outside }
  }
}

// What an instance runs for: a service's `setup`, with its binding, or a call.
pub(super) enum Context {
  Setup(Rc<dyn Db>),
  Call(Rc<ServiceCall>),
}

thread_local! {
  // Everything instances run for on this thread, by id.
  static RUNNING: RefCell<HashMap<u64, Rc<Context>>> = RefCell::default();
  static NEXT: Cell<u64> = const { Cell::new(0) };
}

// Something registered for instances to reach, until it is dropped.
pub(super) struct Running(u64);

impl Running {
  pub(super) fn register(context: Context) -> Running {
    let id = NEXT.with(|next| next.replace(next.get() + 1));
    RUNNING.with(|running| running.borrow_mut().insert(id, Rc::new(context)));
    Running(id)
  }

  pub(super) fn id(&self) -> u64 {
    self.0
  }
}

impl Drop for Running {
  fn drop(&mut self) {
    RUNNING.with(|running| running.borrow_mut().remove(&self.0));
  }
}

// How many things instances run for on this thread.
#[cfg(test)]
pub(super) fn count() -> usize {
  RUNNING.with(|running| running.borrow().len())
}

// What an instance runs for, while it is registered.
fn context(id: u64) -> crate::Result<Rc<Context>> {
  RUNNING.with(|running| running.borrow().get(&id).cloned()).ok_or_else(|| internal("The call this instance ran for has ended"))
}

// The call an instance runs for; `setup` reaches nothing but its binding.
pub(super) fn call(id: u64) -> crate::Result<Rc<ServiceCall>> {
  match context(id)?.as_ref() {
    Context::Call(call) => Ok(Rc::clone(call)),
    Context::Setup(_) => Err(bad_request("A service's setup reaches only its binding")),
  }
}

// How loud a message a service logs is. Only lifted from instances, never built here.
#[allow(dead_code)]
#[derive(ComponentType, Lift, Lower, Clone, Copy, Debug)]
#[component(enum)]
#[repr(u8)]
enum Level {
  #[component(name = "debug")]
  Debug,
  #[component(name = "info")]
  Info,
  #[component(name = "warn")]
  Warn,
  #[component(name = "error")]
  Error,
}

// A failure as it crosses into the instance.
type Crossing<T> = std::result::Result<T, String>;

fn crossing<T>(result: crate::Result<T>) -> Crossing<T> {
  result.map_err(|error| error.to_text())
}

fn params(text: &str) -> crate::Result<Vec<Value>> {
  serde_json::from_str(text).map_err(|error| bad_request(format!("A query's parameters are a JSON array: {error}")))
}

fn run(id: u64, sql: &str, params_text: &str) -> crate::Result<u64> {
  let params = params(params_text)?;
  match context(id)?.as_ref() {
    Context::Setup(db) => db.run(sql, &params),
    Context::Call(call) => call.run(sql, &params),
  }
}

fn all(id: u64, sql: &str, params_text: &str) -> crate::Result<String> {
  let params = params(params_text)?;
  let rows = match context(id)?.as_ref() {
    Context::Setup(db) => db.all(sql, &params),
    Context::Call(call) => call.all(sql, &params),
  }?;
  Ok(Value::from(rows).to_string())
}

fn draw(id: u64) -> crate::Result<String> {
  let call = call(id)?;
  call.op().map(|op| op.draw()).ok_or_else(|| bad_request(format!("{} is not an op, so it draws nothing", call.call.key())))
}

fn log(id: u64, level: Level, message: &str) {
  let service = match context(id).as_deref() {
    Ok(Context::Call(call)) => call.call.service.clone(),
    _ => "setup".into(),
  };
  eprintln!("[{service}] {level:?}: {message}");
}

// Ask the call an instance runs for to call or make an effect, and answer
// once it does, as text.
fn ask(id: u64, input: &str, asking: impl FnOnce(Rc<ServiceCall>, Value, Reply)) -> oneshot::Receiver<Crossing<String>> {
  let (answer, answered) = oneshot::channel();
  let reply: Reply = Box::new(move |result| {
    let _ = answer.send(crossing(result.map(|value| value.to_string())));
  });
  let asked = call(id).and_then(|call| serde_json::from_str(input).map(|input| (call, input)).map_err(|error| bad_request(format!("Input is JSON: {error}"))));
  match asked {
    Ok((call, input)) => asking(call, input, reply),
    Err(error) => reply(Err(error)),
  }
  answered
}

async fn answer(answered: oneshot::Receiver<Crossing<String>>) -> Result<(Crossing<String>,)> {
  Ok((answered.await.unwrap_or_else(|_| Err(internal("The call ended before it answered").to_text())),))
}

fn running(store: &StoreContextMut<'_, Turn>) -> u64 {
  store.data().running
}

// Link the world's host imports.
pub(super) fn link(linker: &mut Linker<Turn>) -> Result<()> {
  let mut host = linker.instance("carbondb:service/host@0.1.0")?;
  host.func_wrap("run", |store, (sql, params): (String, String)| Ok((crossing(run(running(&store), &sql, &params)),)))?;
  host.func_wrap("all", |store, (sql, params): (String, String)| Ok((crossing(all(running(&store), &sql, &params)),)))?;
  host.func_wrap("draw", |store, (): ()| Ok((crossing(draw(running(&store))),)))?;
  host.func_wrap("log", |store, (level, message): (Level, String)| {
    log(running(&store), level, &message);
    Ok(())
  })?;
  host.func_wrap_concurrent("call", |accessor: &Accessor<Turn>, (key, input): (String, String)| {
    let id = accessor.with(|mut access| access.get().running);
    let answered = ask(id, &input, |call, input, reply| HandlerContext::call(call, &key, input, reply));
    Box::pin(answer(answered))
  })?;
  host.func_wrap_concurrent("effect", |accessor: &Accessor<Turn>, (name, input): (String, String)| {
    let id = accessor.with(|mut access| access.get().running);
    let answered = ask(id, &input, |call, input, reply| HandlerContext::effect(call, &name, input, reply));
    Box::pin(answer(answered))
  })?;
  Ok(())
}
