//! The order work runs in on one side of a space: one job at a time, first in,
//! first out. A space has a queue for its writer and one for its reader, so a
//! transaction never overlaps another on the same connection. Local calls go
//! through it today; synced ops will join the same queue.
//!
//! The queue only decides what runs next. Whoever drives it starts each job it
//! hands back, and reports when that job is done.

use std::collections::VecDeque;

pub struct Queue<J> {
  running: bool,
  waiting: VecDeque<J>,
}

impl<J> Default for Queue<J> {
  fn default() -> Self {
    Queue { running: false, waiting: VecDeque::new() }
  }
}

impl<J> Queue<J> {
  // Take a job: hand it back to start now when nothing is running, or keep it
  // behind the jobs already waiting.
  pub fn push(&mut self, job: J) -> Option<J> {
    if self.running {
      self.waiting.push_back(job);
      None
    } else {
      self.running = true;
      Some(job)
    }
  }

  // The running job is done: hand back the next one to start, if any.
  pub fn finish(&mut self) -> Option<J> {
    let next = self.waiting.pop_front();
    self.running = next.is_some();
    next
  }

  pub fn is_running(&self) -> bool {
    self.running
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn starts_a_job_at_once_only_when_nothing_is_running() {
    let mut queue = Queue::default();
    assert_eq!(queue.push("a"), Some("a"));
    assert_eq!(queue.push("b"), None);
    assert!(queue.is_running());
  }

  #[test]
  fn hands_back_waiting_jobs_in_the_order_pushed_then_goes_idle() {
    let mut queue = Queue::default();
    queue.push("a");
    queue.push("b");
    queue.push("c");
    assert_eq!([queue.finish(), queue.finish(), queue.finish()], [Some("b"), Some("c"), None]);
    assert!(!queue.is_running());
    assert_eq!(queue.push("d"), Some("d"));
  }
}
