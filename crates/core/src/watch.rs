//! Watches: a GET that runs now and again after every commit that writes a
//! table it read, handing each answer to whoever watches, and the listeners of
//! a space's RunState. The core keeps both; a runtime only registers them and
//! hands each answer to the app.
//!
//! A run's snapshot is taken when it starts, but its read set is known only
//! when it answers, so a commit landing in between cannot be judged on
//! arrival. While a run is in flight, the tables such commits wrote are kept,
//! and judged against the fresh read set when the run answers. The rule, then:
//! a watch runs again exactly when a commit after its last run started wrote a
//! table that run read.
//!
//! Stopping or resuming a space changes what every GET answers, so every
//! watch on it runs again. A call from the app is answered only once the
//! watches on its space have settled, so after a call every watched view
//! already shows it.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use serde_json::Value;

use crate::carbon::Carbon;
use crate::dispatch::{Reply, Settled};
use crate::error::Result;
use crate::space::{RunState, Space};
use crate::{Actor, Call, Routing};

// Stops a watch or a RunState listener.
pub type WatchId = u32;

// Hands a watch each answer: its GET's value, or its error.
pub type Delivery = Rc<dyn Fn(Result<Value>)>;

// Called with a space's RunState each time it changes.
pub type RunStateListener = Rc<dyn Fn(&RunState)>;

// A call from the app waiting for the watches on its space to settle.
type Waiting = Box<dyn FnOnce()>;

// When a watch must run again, as data: `read`, the read set of its last
// answer; `stale`, whether it must run again; and `in_flight`, the tables
// written since the running run's snapshot, or none when no run is in flight.
#[derive(Debug, Clone, PartialEq)]
struct Reads {
  read: BTreeSet<String>,
  stale: bool,
  in_flight: Option<BTreeSet<String>>,
}

impl Reads {
  // A new watch, which runs first.
  fn unseen() -> Reads {
    Reads { read: BTreeSet::new(), stale: true, in_flight: None }
  }

  // A run starts: it reads everything committed so far.
  fn started(&self) -> Reads {
    Reads { stale: false, in_flight: Some(BTreeSet::new()), ..self.clone() }
  }

  // A commit wrote these tables.
  fn committed(&self, tables: &[String]) -> Reads {
    match &self.in_flight {
      Some(in_flight) => Reads { in_flight: Some(in_flight.iter().chain(tables).cloned().collect()), ..self.clone() },
      None => Reads { stale: self.stale || meets(&self.read, tables.iter()), ..self.clone() },
    }
  }

  // The run in flight finished, with the tables it read; a failed GET reports
  // none, and the last read set stands.
  fn finished(&self, tables: Option<&[String]>) -> Reads {
    let read = tables.map_or_else(|| self.read.clone(), |tables| tables.iter().cloned().collect());
    let stale = self.stale || meets(&read, self.in_flight.iter().flatten());
    Reads { read, stale, in_flight: None }
  }

  // Something changed what every GET answers, such as the space stopping.
  fn outdated(&self) -> Reads {
    Reads { stale: true, ..self.clone() }
  }
}

fn meets<'a>(read: &BTreeSet<String>, mut tables: impl Iterator<Item = &'a String>) -> bool {
  tables.any(|table| read.contains(table))
}

// One watch: who reads what in which space, and where its runs stand.
struct Watch {
  space: String,
  actor: Actor,
  call: Call,
  deliver: Delivery,
  reads: RefCell<Reads>,
  running: Cell<bool>,
  stopped: Cell<bool>,
}

impl Watch {
  // Whether it has run against every commit so far, or no longer runs.
  fn settled(&self) -> bool {
    self.stopped.get() || (!self.running.get() && !self.reads.borrow().stale)
  }
}

// Every watch and RunState listener on this device, and whoever waits for a
// space's watches to settle.
#[derive(Default)]
pub struct Watches {
  next: Cell<WatchId>,
  gets: RefCell<BTreeMap<WatchId, Rc<Watch>>>,
  run_states: RefCell<BTreeMap<WatchId, (String, RunStateListener)>>,
  waiting: RefCell<HashMap<String, Vec<Waiting>>>,
}

impl Watches {
  fn id(&self) -> WatchId {
    let id = self.next.get();
    self.next.set(id + 1);
    id
  }

  fn on(&self, space: &str) -> Vec<Rc<Watch>> {
    self.gets.borrow().values().filter(|watch| watch.space == space).cloned().collect()
  }

  fn listening(&self, space: &str) -> Vec<RunStateListener> {
    self.run_states.borrow().values().filter(|(on, _)| on == space).map(|(_, listener)| Rc::clone(listener)).collect()
  }

  fn settled(&self, space: &str) -> bool {
    self.on(space).iter().all(|watch| watch.settled())
  }
}

impl Carbon {
  // Run a GET on a space now, by its service key and as the identity named, and
  // again after every commit that writes a table it read, handing each answer
  // to `deliver`. The id stops it.
  pub fn watch(self: &Rc<Self>, space: &str, identity: Option<&str>, key: &str, deliver: Delivery) -> Result<WatchId> {
    let watch = Rc::new(Watch {
      space: space.to_string(),
      actor: self.actor(identity)?,
      call: Call::from_key(key)?,
      deliver,
      reads: RefCell::new(Reads::unseen()),
      running: Cell::new(false),
      stopped: Cell::new(false),
    });
    let id = self.watches.id();
    self.watches.gets.borrow_mut().insert(id, Rc::clone(&watch));
    self.rerun(&watch);
    Ok(id)
  }

  // Hear every change of a space's RunState, whether or not it is open yet. The id stops it.
  pub fn watch_run_state(&self, space: &str, listener: RunStateListener) -> WatchId {
    let id = self.watches.id();
    self.watches.run_states.borrow_mut().insert(id, (space.to_string(), listener));
    id
  }

  // Stop a watch or a RunState listener.
  pub fn unwatch(&self, id: WatchId) {
    self.watches.run_states.borrow_mut().remove(&id);
    let watch = self.watches.gets.borrow_mut().remove(&id);
    if let Some(watch) = watch {
      watch.stopped.set(true);
      self.settle(&watch.space);
    }
  }

  // Answer once the watches on a space have settled.
  pub(crate) fn when_settled(&self, space: &str, answer: Waiting) {
    if self.watches.settled(space) {
      return answer();
    }
    self.watches.waiting.borrow_mut().entry(space.to_string()).or_default().push(answer);
  }

  // A commit to a space wrote these tables: each watch that read one runs again.
  pub(crate) fn committed(self: &Rc<Self>, space: &str, tables: &[String]) {
    for watch in self.watches.on(space) {
      watch.reads.replace_with(|reads| reads.committed(tables));
      self.rerun(&watch);
    }
  }

  // Move a space to a RunState and tell whoever listens. Stopping or resuming
  // changes what every GET answers, so every watch on it runs again.
  pub(crate) fn set_run_state(self: &Rc<Self>, space: &Space, state: RunState) {
    let stopping_or_resuming = is_stopped(&space.run_state()) != is_stopped(&state);
    space.set_run_state(state.clone());
    for listener in self.watches.listening(&space.id) {
      listener(&state);
    }
    if stopping_or_resuming {
      for watch in self.watches.on(&space.id) {
        watch.reads.replace_with(|reads| reads.outdated());
        self.rerun(&watch);
      }
    }
  }

  // Run a watch's GET when a commit since its last run started wrote what it read.
  fn rerun(self: &Rc<Self>, watch: &Rc<Watch>) {
    if watch.stopped.get() || watch.running.get() || !watch.reads.borrow().stale {
      return self.settle(&watch.space);
    }
    watch.running.set(true);
    watch.reads.replace_with(|reads| reads.started());
    let (carbon, running) = (Rc::clone(self), Rc::clone(watch));
    let answered: Reply = Box::new(move |settled| carbon.answered(&running, settled));
    let (carbon, actor, call) = (Rc::clone(self), watch.actor.clone(), watch.call.clone());
    self.with_space(&watch.space, Box::new(move |space| match space {
      Ok(space) => carbon.submit(&space, actor, call, Value::Null, answered),
      Err(error) => answered(Err(error)),
    }));
  }

  // A watch's GET answered: hand over the answer, then run again if a commit
  // since it started wrote what it read.
  fn answered(self: &Rc<Self>, watch: &Rc<Watch>, settled: Result<Settled>) {
    watch.running.set(false);
    if watch.stopped.get() {
      return self.settle(&watch.space);
    }
    watch.reads.replace_with(|reads| reads.finished(settled.as_ref().ok().map(|settled| settled.tables.as_slice())));
    (watch.deliver)(settled.map(|settled| settled.value));
    self.rerun(watch);
  }

  // Answer whoever waits on a space's watches, once every one has settled.
  fn settle(&self, space: &str) {
    if !self.watches.settled(space) {
      return;
    }
    let waiting = self.watches.waiting.borrow_mut().remove(space).unwrap_or_default();
    for answer in waiting {
      answer();
    }
  }
}

fn is_stopped(state: &RunState) -> bool {
  matches!(state, RunState::Stopped { .. })
}

#[cfg(test)]
mod tests {
  use super::*;

  const WRITES: [&[&str]; 3] = [&["note"], &["tag"], &["note", "tag"]];
  const READ_SETS: [&[&str]; 4] = [&[], &["note"], &["tag"], &["note", "tag"]];

  #[derive(Clone, Copy, Debug, PartialEq)]
  enum Slot {
    Before,
    During,
    After,
  }

  type Commit = (&'static [&'static str], Slot);

  fn tables(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
  }

  // Every list of up to `n` commits, each with the tables it wrote and when it landed.
  fn commit_lists(n: usize) -> Vec<Vec<Commit>> {
    if n == 0 {
      return vec![vec![]];
    }
    let shorter = commit_lists(n - 1);
    let longer: Vec<Vec<Commit>> = shorter
      .iter()
      .filter(|list| list.len() == n - 1)
      .flat_map(|list| WRITES.iter().flat_map(move |written| [Slot::Before, Slot::During, Slot::After].map(|slot| [list.clone(), vec![(*written, slot)]].concat())))
      .collect();
    [shorter, longer].concat()
  }

  // The watch after its previous answer, one run, and the commits around it.
  fn replay(previous: &[&str], fresh: &[&str], commits: &[Commit]) -> Reads {
    let at = |slot: Slot| commits.iter().filter(move |(_, landed)| *landed == slot).map(|(written, _)| tables(written));
    let mut reads = Reads::unseen().started().finished(Some(&tables(previous)));
    for written in at(Slot::Before) {
      reads = reads.committed(&written);
    }
    reads = reads.started();
    for written in at(Slot::During) {
      reads = reads.committed(&written);
    }
    reads = reads.finished(Some(&tables(fresh)));
    for written in at(Slot::After) {
      reads = reads.committed(&written);
    }
    reads
  }

  // The rule itself: run again exactly when a commit after the run started wrote a table the run read.
  fn model(fresh: &[&str], commits: &[Commit]) -> bool {
    commits.iter().any(|(written, slot)| *slot != Slot::Before && written.iter().any(|table| fresh.contains(table)))
  }

  #[test]
  fn runs_again_exactly_when_a_commit_after_its_run_started_wrote_a_table_that_run_read() {
    let mut cases = 0;
    for previous in READ_SETS {
      for fresh in READ_SETS {
        for commits in commit_lists(3) {
          assert_eq!(replay(previous, fresh, &commits).stale, model(fresh, &commits), "{previous:?} {fresh:?} {commits:?}");
          cases += 1;
        }
      }
    }
    assert_eq!(cases, 16 * (1 + 9 + 81 + 729));
  }

  // What each watch on the personal space has been handed, by its GET.
  #[test]
  fn hands_over_its_get_now_and_after_each_commit_that_wrote_what_it_read_until_stopped() {
    use crate::testing::{alice, carbon, now};
    use serde_json::json;

    let (_directory, carbon) = carbon();
    let identity = alice(&carbon);
    let watching = |key: &str| {
      let handed = Rc::new(RefCell::new(vec![]));
      let into = Rc::clone(&handed);
      let id = carbon.watch(&identity.personal, Some(&identity.id), key, Rc::new(move |answer: Result<Value>| into.borrow_mut().push(answer.unwrap()))).unwrap();
      (id, handed)
    };
    let post = |text: &str| now(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), "POST [notes]/run", json!({"do": [{"insert": text}]}), answer)).unwrap();
    let (texts, notes) = watching("GET [notes]/texts");
    let (_refs, refs) = watching("GET [space]/refs");
    post("a");
    carbon.unwatch(texts);
    post("b");
    assert_eq!(*notes.borrow(), [json!([]), json!(["a"])]);
    assert_eq!(refs.borrow().len(), 1);
  }

  #[test]
  fn runs_first_keeps_its_read_set_when_a_run_fails_and_runs_again_once_outdated() {
    assert!(Reads::unseen().stale);
    let read = Reads::unseen().started().finished(Some(&tables(&["note"])));
    let failed = read.started().committed(&tables(&["note"])).finished(None);
    assert_eq!((failed.read, failed.stale), (BTreeSet::from(["note".to_string()]), true));
    assert!(Reads::unseen().started().finished(Some(&[])).outdated().stale);
  }
}
