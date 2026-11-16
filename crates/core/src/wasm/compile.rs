//! Compiling components to this machine's code, which is slow: about two
//! seconds for a JavaScript component, spread across cores, so it never runs
//! on the core's thread.
//!
//! - **One engine per process.** Every component compiles and runs on it, so
//!   compiled code is shared by every CarbonDB the process opens.
//! - **Kept on disk.** Wasmtime's cache keeps compiled code in the directory
//!   of the first CarbonDB the process opens (`compiled` in its config,
//!   beside the system database by default), keyed by the component's bytes,
//!   the engine's settings and wasmtime's version; a restart loads what it
//!   compiled before. Loading compiled code trusts that directory as the
//!   device trusts the rest of its data directory, its keys included.
//! - **A pool of its own.** Two threads only compile, each compile spread
//!   across cores by wasmtime; a compiled component crosses back to whoever
//!   waits on it.
//! - **Once per version.** Every package of the same version shares one compile.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;
use std::time::Duration;

use tokio::sync::watch;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Cache, CacheConfig, Config, Engine};

use super::host::{self, Turn};
use crate::error::{Result, ServiceError, bad_request, internal};

// How often an instance that computes without yielding is made to yield.
const TICK: Duration = Duration::from_millis(10);

// How many components compile at once.
const THREADS: usize = 2;

// Where compiled code is kept, set by the first CarbonDB the process opens.
static KEPT_IN: OnceLock<PathBuf> = OnceLock::new();

// The engine every component compiles and runs on, ticking its epoch.
pub(super) static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
  let engine = engine(KEPT_IN.get().map(PathBuf::as_path)).expect("the host's configuration is valid").0;
  let ticking = engine.weak();
  std::thread::spawn(move || {
    while let Some(engine) = ticking.upgrade() {
      engine.increment_epoch();
      drop(engine);
      std::thread::sleep(TICK);
    }
  });
  engine
});

// What every instance links: the world's host imports, WASI, and HTTP through the core.
static LINKER: LazyLock<Linker<Turn>> = LazyLock::new(|| {
  let mut linker = Linker::new(&ENGINE);
  host::link(&mut linker).expect("the host imports link");
  wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("WASI 0.2 links");
  wasmtime_wasi::p3::add_to_linker(&mut linker).expect("WASI 0.3 links");
  wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).expect("WASI 0.2 HTTP links");
  wasmtime_wasi_http::p3::add_to_linker(&mut linker).expect("WASI 0.3 HTTP links");
  linker
});

// Every component compiled or compiling in this process, by version.
static COMPILED: LazyLock<Mutex<HashMap<String, Compiled>>> = LazyLock::new(Mutex::default);

// The threads that compile, taking work in turn.
static POOL: LazyLock<Mutex<mpsc::Sender<Job>>> = LazyLock::new(|| {
  let (jobs, queue) = mpsc::channel::<Job>();
  let queue = Arc::new(Mutex::new(queue));
  for _ in 0..THREADS {
    let queue = Arc::clone(&queue);
    std::thread::Builder::new()
      .name("carbondb-compile".into())
      .spawn(move || {
        while let Ok(job) = queue.lock().expect("the compile queue is never poisoned").recv() {
          job();
        }
      })
      .expect("a compile thread starts");
  }
  Mutex::new(jobs)
});

type Job = Box<dyn FnOnce() + Send>;

// Keep compiled code in `directory`, unless the engine already exists: the
// first CarbonDB a process opens chooses for every one it opens after.
pub fn keep_in(directory: &Path) {
  let _ = KEPT_IN.set(directory.to_path_buf());
}

// An engine for components, keeping its compiled code in `directory` when
// given one, and the cache it keeps it in.
pub(super) fn engine(directory: Option<&Path>) -> Result<(Engine, Option<Cache>)> {
  let mut config = Config::new();
  config.wasm_component_model_async(true).epoch_interruption(true);
  let cache = directory.map(cache).transpose()?;
  config.cache(cache.clone());
  let engine = Engine::new(&config).map_err(|error| internal(format!("Cannot start the WASM host: {error}")))?;
  Ok((engine, cache))
}

fn cache(directory: &Path) -> Result<Cache> {
  let mut config = CacheConfig::new();
  config.with_directory(directory);
  Cache::new(config).map_err(|error| bad_request(format!("Cannot keep compiled components in {}: {error}", directory.display())))
}

// A component compiled once per process, by version, on the pool: what is
// already compiled or compiling is shared.
pub(super) fn compiled(version: &str, bytes: &[u8]) -> Compiled {
  let mut compiled = COMPILED.lock().expect("the compile table is never poisoned");
  if let Some(found) = compiled.get(version) {
    return found.clone();
  }
  let (done, compiling) = watch::channel(None);
  let bytes = bytes.to_vec();
  let job: Job = Box::new(move || {
    done.send_replace(Some(compile(&bytes)));
  });
  POOL.lock().expect("the compile pool is never poisoned").send(job).expect("the compile threads run as long as the process");
  compiled.insert(version.to_string(), Compiled(compiling.clone()));
  Compiled(compiling)
}

// Compile a component and link it, ready to instantiate.
pub(super) fn compile(bytes: &[u8]) -> std::result::Result<InstancePre<Turn>, ServiceError> {
  let component = Component::new(&ENGINE, bytes).map_err(|error| bad_request(format!("Not a component this host can run: {error}")))?;
  LINKER.instantiate_pre(&component).map_err(|error| bad_request(format!("A component of another world than carbondb:service: {error}")))
}

// A component compiling on the pool, or compiled.
#[derive(Clone)]
pub struct Compiled(watch::Receiver<Option<Result<InstancePre<Turn>>>>);

impl Compiled {
  // The component, once compiled; none while it compiles.
  pub(super) fn now(&self) -> Option<Result<InstancePre<Turn>>> {
    self.0.borrow().clone()
  }

  // Wait here until it has compiled, or failed to.
  pub fn wait(self) -> Result<()> {
    block_on(self.ready())
  }

  // Once it has compiled, or failed to.
  pub async fn ready(mut self) -> Result<()> {
    let done = self.0.wait_for(Option::is_some).await.map_err(|_| internal("The compile pool stopped"))?;
    done.as_ref().expect("compiled").as_ref().map(drop).map_err(Clone::clone)
  }
}

// Run a future to completion on this thread, sleeping while it waits.
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
  let mut future = pin!(future);
  let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
  let mut context = Context::from_waker(&waker);
  loop {
    if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
      return output;
    }
    std::thread::park();
  }
}

struct Unpark(Thread);

impl Wake for Unpark {
  fn wake(self: Arc<Self>) {
    self.0.unpark();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testing::{PROBE, afresh};

  #[test]
  fn a_version_compiles_once_per_process() {
    let bytes = afresh(PROBE);
    let (first, second) = (compiled("once", &bytes), compiled("once", &bytes));
    assert!(first.0.same_channel(&second.0), "the second waits on the first's compile");
    first.wait().unwrap();
  }

  #[test]
  fn compiled_code_is_kept_on_disk_for_the_next_engine() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = afresh(PROBE);
    let (before, kept) = engine(Some(directory.path())).unwrap();
    Component::new(&before, &bytes).unwrap();
    let (after, loaded) = engine(Some(directory.path())).unwrap();
    Component::new(&after, &bytes).unwrap();
    let (kept, loaded) = (kept.unwrap(), loaded.unwrap());
    assert_eq!((kept.cache_hits(), kept.cache_misses()), (0, 1), "compiled and kept");
    assert_eq!((loaded.cache_hits(), loaded.cache_misses()), (1, 0), "loaded, not compiled");
  }
}
