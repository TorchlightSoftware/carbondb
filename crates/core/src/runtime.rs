//! The core on a thread of its own. CarbonDB is single-threaded: every space,
//! service call and component instance lives on one thread, driven by a
//! single-threaded tokio runtime, whose timers are the core's, whose local
//! tasks run its components' calls, and whose network carries the requests
//! services make. Everything else reaches it by message:
//! `Runtime` is a handle any thread may hold, which sends the core work to do
//! there. Work answers through callbacks the caller makes thread-safe, or,
//! with `ask`, the caller waits for its answer.
//!
//! The thread runs as long as a handle does; when the last is dropped, the
//! core closes with it.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use tokio::sync::mpsc as queue;
use tokio::task::LocalSet;

use crate::carbon::{Cancel, Carbon, Schedule, Spawn};
use crate::error::{Result, internal};

// Work for the core, done on its thread.
type Work = Box<dyn FnOnce(&Rc<Carbon>) + Send>;

#[derive(Clone)]
pub struct Runtime {
  work: queue::UnboundedSender<Work>,
}

impl Runtime {
  // Open CarbonDB from its config on a thread of its own, once it has opened.
  pub fn start(config: &Path) -> Result<Runtime> {
    let (work, queued) = queue::unbounded_channel();
    let (opened, opening) = mpsc::channel();
    let config = config.to_path_buf();
    std::thread::Builder::new()
      .name("carbondb".into())
      .spawn(move || run(config, queued, opened))
      .map_err(|error| internal(format!("Cannot start CarbonDB's thread: {error}")))?;
    opening.recv().map_err(|_| internal("CarbonDB's thread stopped as it opened"))??;
    Ok(Runtime { work })
  }

  // Do something on the core's thread; it answers through whatever it was given.
  pub fn with(&self, work: impl FnOnce(&Rc<Carbon>) + Send + 'static) {
    // The thread outlives every handle, so it is always there to take work.
    let _ = self.work.send(Box::new(work));
  }

  // Do something on the core's thread, and wait here for its answer.
  pub fn ask<T: Send + 'static>(&self, work: impl FnOnce(&Rc<Carbon>) -> Result<T> + Send + 'static) -> Result<T> {
    let (answer, answered) = mpsc::channel();
    self.with(move |carbon| {
      let _ = answer.send(work(carbon));
    });
    answered.recv().map_err(|_| internal("CarbonDB's thread stopped before it answered"))?
  }
}

// The core's thread: open CarbonDB, say so, then do the work sent to it until
// every handle is gone.
fn run(config: PathBuf, mut queued: queue::UnboundedReceiver<Work>, opened: mpsc::Sender<Result<()>>) {
  let tokio = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
    Ok(tokio) => tokio,
    Err(error) => {
      let _ = opened.send(Err(internal(format!("Cannot start CarbonDB's runtime: {error}"))));
      return;
    }
  };
  let local = LocalSet::new();
  local.block_on(&tokio, async move {
    let carbon = match Carbon::open(&config, timers(), tasks()) {
      Ok(carbon) => carbon,
      Err(error) => {
        let _ = opened.send(Err(error));
        return;
      }
    };
    let _ = opened.send(Ok(()));
    while let Some(work) = queued.recv().await {
      work(&carbon);
    }
  });
}

// The core's timers: tokio's, on its thread.
fn timers() -> Schedule {
  Rc::new(|delay: Duration, then: Box<dyn FnOnce()>| {
    let timer = tokio::task::spawn_local(async move {
      tokio::time::sleep(delay).await;
      then();
    });
    Box::new(move || timer.abort()) as Cancel
  })
}

// The core's tasks: local tasks on its thread.
fn tasks() -> Spawn {
  Rc::new(|task| {
    let running = tokio::task::spawn_local(task);
    Box::new(move || running.abort()) as Cancel
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{Value, json};

  fn config() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("carbondb.toml");
    std::fs::write(&path, "system = \"data/system.db\"\n[sync]\nmode = \"none\"\n").unwrap();
    (directory, path)
  }

  // A call made from this thread, answered from the core's.
  fn call(runtime: &Runtime, key: &'static str, input: Value) -> Result<Value> {
    let (answer, answered) = mpsc::channel();
    runtime.with(move |carbon| carbon.call(None, None, key, input, Box::new(move |result| answer.send(result).unwrap())));
    answered.recv().unwrap()
  }

  #[test]
  fn the_core_answers_calls_sent_from_another_thread_on_its_own() {
    let (_directory, path) = config();
    let runtime = Runtime::start(&path).unwrap();
    let founded = call(&runtime, "POST [@carbon]/identity", json!({"slug": "alice", "device": "Pixel9"})).unwrap();
    let elsewhere = runtime.clone();
    let listed = std::thread::spawn(move || call(&elsewhere, "GET [@carbon]/identities", Value::Null)).join().unwrap().unwrap();
    assert_eq!(listed[0]["id"], founded["id"]);
    let personal = runtime.ask(|carbon| Ok(carbon.identity("alice")?.personal)).unwrap();
    assert_eq!(json!(personal), founded["personal"]);
  }

  #[test]
  fn a_config_it_cannot_open_is_refused_from_start() {
    let directory = tempfile::tempdir().unwrap();
    let error = Runtime::start(&directory.path().join("missing.toml")).err().unwrap();
    assert_eq!(error.status, 400);
  }
}
