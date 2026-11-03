//! The op clock. Single-node needs only a time that never repeats or goes
//! backwards on this device; replicated mode makes it a hybrid logical clock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Clock {
  last: AtomicU64,
}

impl Clock {
  // Milliseconds since the epoch, or one past the last time given if the wall clock has not moved on.
  pub fn now(&self) -> u64 {
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or_default();
    let previous = self.last.try_update(Ordering::SeqCst, Ordering::SeqCst, |last| Some(wall.max(last + 1))).unwrap_or_default();
    wall.max(previous + 1)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn times_always_increase() {
    let clock = Clock::default();
    let times: Vec<u64> = (0..1000).map(|_| clock.now()).collect();
    assert!(times.windows(2).all(|pair| pair[1] > pair[0]));
  }
}
