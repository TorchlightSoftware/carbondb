//! A space's RunQueue: the ops waiting to apply there, local calls and synced
//! ops alike, applied one at a time in the order they arrived. Reads never
//! wait in it.
//!
//! The queue only decides what runs next. Whoever drains it applies each op it
//! hands out, and reports when that op is done. An op may finish while it is
//! being started, so draining is a loop rather than a recursion: an op that
//! finishes at once hands the turn straight to the next.

use std::collections::VecDeque;

pub struct RunQueue<J> {
  waiting: VecDeque<J>,
  running: bool,
  draining: bool,
}

impl<J> Default for RunQueue<J> {
  fn default() -> Self {
    RunQueue { waiting: VecDeque::new(), running: false, draining: false }
  }
}

impl<J> RunQueue<J> {
  pub fn push(&mut self, job: J) {
    self.waiting.push_back(job);
  }

  // The next op to apply, when none is running; it runs until `finish`.
  pub fn take(&mut self) -> Option<J> {
    if self.running {
      return None;
    }
    let next = self.waiting.pop_front();
    self.running = next.is_some();
    next
  }

  // The running op is done.
  pub fn finish(&mut self) {
    self.running = false;
  }

  pub fn is_running(&self) -> bool {
    self.running
  }

  // Claim the drain loop: false when it is already running further up the stack.
  pub fn begin_drain(&mut self) -> bool {
    !std::mem::replace(&mut self.draining, true)
  }

  pub fn end_drain(&mut self) {
    self.draining = false;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hands_out_one_op_at_a_time_in_the_order_pushed() {
    let mut queue = RunQueue::default();
    queue.push("a");
    queue.push("b");
    assert_eq!(queue.take(), Some("a"));
    assert_eq!(queue.take(), None);
    queue.finish();
    assert_eq!(queue.take(), Some("b"));
    queue.finish();
    assert_eq!((queue.take(), queue.is_running()), (None, false));
  }

  #[test]
  fn only_one_drain_loop_runs_at_a_time() {
    let mut queue = RunQueue::<()>::default();
    assert!(queue.begin_drain());
    assert!(!queue.begin_drain());
    queue.end_drain();
    assert!(queue.begin_drain());
  }
}
