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
//! watch on it runs again. A write from the app is answered only once the
//! watches its commit owes a run have run, so after the call every watched
//! view already shows it: those that read a table it wrote, and those in
//! flight when it landed, which are judged when they answer. A GET commits
//! nothing, so it answers at once.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
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

// A call from the app waiting for the watches its commit owes a run.
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

  // Whether a run is still owed: it must run again, or the run in flight is
  // judged only when it answers.
  fn owed(&self) -> bool {
    self.stale || self.in_flight.is_some()
  }

  // Whether to start a run now: it must run again, and none is in flight.
  fn due(&self) -> bool {
    self.stale && self.in_flight.is_none()
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
  stopped: Cell<bool>,
}

impl Watch {
  // Whether it has run against every commit so far, or no longer runs.
  fn settled(&self) -> bool {
    self.stopped.get() || !self.reads.borrow().owed()
  }
}

// Every watch and RunState listener on this device, and the calls waiting for
// the watches their commits owe a run.
#[derive(Default)]
pub struct Watches {
  next: Cell<WatchId>,
  gets: RefCell<BTreeMap<WatchId, Rc<Watch>>>,
  run_states: RefCell<BTreeMap<WatchId, (String, RunStateListener)>>,
  waiting: RefCell<Vec<(Vec<WatchId>, Waiting)>>,
}

impl Watches {
  fn id(&self) -> WatchId {
    let id = self.next.get();
    self.next.set(id + 1);
    id
  }

  fn on(&self, space: &str) -> Vec<(WatchId, Rc<Watch>)> {
    self.gets.borrow().iter().filter(|(_, watch)| watch.space == space).map(|(id, watch)| (*id, Rc::clone(watch))).collect()
  }

  fn listening(&self, space: &str) -> Vec<RunStateListener> {
    self.run_states.borrow().values().filter(|(on, _)| on == space).map(|(_, listener)| Rc::clone(listener)).collect()
  }

  // Whether each of these watches has run, or no longer runs.
  fn ran(&self, ids: &[WatchId]) -> bool {
    let gets = self.gets.borrow();
    ids.iter().all(|id| gets.get(id).is_none_or(|watch| watch.settled()))
  }

  // The calls whose watches have all run, no longer waiting.
  fn ready(&self) -> Vec<Waiting> {
    let waiting = self.waiting.take();
    let (ready, still): (Vec<_>, Vec<_>) = waiting.into_iter().partition(|(ids, _)| self.ran(ids));
    self.waiting.replace(still);
    ready.into_iter().map(|(_, answer)| answer).collect()
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
      self.settle();
    }
  }

  // Answer once these watches have run.
  pub(crate) fn when_run(&self, watches: Vec<WatchId>, answer: Waiting) {
    if self.watches.ran(&watches) {
      return answer();
    }
    self.watches.waiting.borrow_mut().push((watches, answer));
  }

  // A commit to a space wrote these tables: each watch that read one runs
  // again. Returns the watches it owes a run.
  pub(crate) fn committed(self: &Rc<Self>, space: &str, tables: &[String]) -> Vec<WatchId> {
    let watches = self.watches.on(space);
    let owed = watches
      .iter()
      .filter_map(|(id, watch)| {
        watch.reads.replace_with(|reads| reads.committed(tables));
        watch.reads.borrow().owed().then_some(*id)
      })
      .collect();
    for (_, watch) in &watches {
      self.rerun(watch);
    }
    owed
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
      for (_, watch) in self.watches.on(&space.id) {
        watch.reads.replace_with(|reads| reads.outdated());
        self.rerun(&watch);
      }
    }
  }

  // Run a watch's GET when a commit since its last run started wrote what it read.
  fn rerun(self: &Rc<Self>, watch: &Rc<Watch>) {
    if watch.stopped.get() || !watch.reads.borrow().due() {
      return self.settle();
    }
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
    if watch.stopped.get() {
      return self.settle();
    }
    watch.reads.replace_with(|reads| reads.finished(settled.as_ref().ok().map(|settled| settled.tables.as_slice())));
    (watch.deliver)(settled.map(|settled| settled.value));
    self.rerun(watch);
  }

  // Answer the calls whose watches have all run.
  fn settle(&self) {
    for answer in self.watches.ready() {
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
  fn a_write_waits_for_the_watches_its_commit_owes_a_run_and_a_get_for_none() {
    use crate::testing::{actor, alice, call_now, later, space_now, timed};
    use serde_json::json;

    let (_directory, carbon, timers) = timed();
    let identity = alice(&carbon);
    let personal = space_now(&carbon, &identity.personal).unwrap();
    let (space, acting) = (Some(identity.personal.as_str()), Some(identity.id.as_str()));
    let answers = Rc::new(RefCell::new(vec![]));
    let watching = |key: &str| {
      let into = Rc::clone(&answers);
      carbon.watch(&identity.personal, acting, key, Rc::new(move |answer: Result<Value>| into.borrow_mut().push(answer.map_err(|error| error.status)))).unwrap()
    };
    let insert = |text: &str| json!({"do": [{"insert": text}]});
    let (texts, _refs) = (watching("GET [notes]/texts"), watching("GET [space]/refs"));
    let post = |text: &str| call_now(&carbon, &personal, &actor(&identity), "POST [notes]/run", insert(text)).unwrap();
    assert_eq!(post("a").watches, [texts], "only the watch that read what it wrote");
    assert_eq!(call_now(&carbon, &personal, &actor(&identity), "GET [notes]/texts", Value::Null).unwrap().watches, Vec::<WatchId>::new());

    watching("GET [notes]/hang");
    let write = later(|answer| carbon.call(space, acting, "POST [notes]/run", insert("b"), answer));
    let read = later(|answer| carbon.call(space, acting, "GET [notes]/texts", Value::Null, answer));
    assert!(write.borrow().is_none(), "a write waits for a watch in flight, judged only when it answers");
    assert_eq!(read.borrow_mut().take().unwrap().unwrap(), json!(["a", "b"]), "a GET answers at once");
    timers.run();
    assert_eq!(answers.borrow().last(), Some(&Err(504)), "the hung watch times out");
    assert!(write.borrow_mut().take().unwrap().is_ok(), "which frees the write");
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
