//! The op clock: a hybrid logical clock in milliseconds. A time never repeats
//! or goes backwards on this device, and every time it gives is later than
//! every op this device has received, so an op always sorts after every op its
//! author had seen. The shared order is therefore one consistent flattening of
//! the causal order p2panda delivers ops in, and `time` stays one millisecond
//! integer that handlers store as a timestamp.
//!
//! A received op's time is admitted before the clock takes it in. A time no
//! device could have made is refused for good, the same on every replica. A
//! time too far ahead of this device's wall clock is refused only for now,
//! since another device's wall clock may already have reached it: refusing it
//! for good would leave replicas holding different ops. Its sender offers it
//! again later, and one peer's fast clock cannot drag every replica's times
//! forward.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Result, ServiceError, bad_request};

// The first time no op may have: JavaScript reads integers exactly only below
// 2^53, and SQLite stores integers only below 2^63.
pub const NEVER: u64 = 1 << 53;

// How far ahead of this device's wall clock a received op's time may be: five
// minutes, room for devices whose wall clocks drift.
pub const AHEAD: u64 = 5 * 60 * 1000;

#[derive(Default)]
pub struct Clock {
  last: AtomicU64,
}

impl Clock {
  // The wall clock in milliseconds, or one past the latest time given or
  // observed if the wall clock has not moved past it.
  pub fn now(&self) -> u64 {
    self.next(wall())
  }

  // Take in the time of an op received from another device and admitted, so
  // the next time this device gives sorts after it.
  pub fn observe(&self, time: u64) {
    self.last.fetch_max(time, Ordering::SeqCst);
  }

  // The time to give when the wall clock reads `wall`.
  fn next(&self, wall: u64) -> u64 {
    let previous = self.last.try_update(Ordering::SeqCst, Ordering::SeqCst, |last| Some(wall.max(last + 1))).unwrap_or_default();
    wall.max(previous + 1)
  }
}

// This device's wall clock, in milliseconds.
pub fn wall() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or_default()
}

// Whether a received op's time may enter the op log when this device's wall
// clock reads `wall`: refused for good at or past `NEVER`, and for now when
// more than `AHEAD` ahead of the wall clock.
pub fn admit(time: u64, wall: u64) -> Result<()> {
  if time >= NEVER {
    return Err(bad_request(format!("An op's time is below 2^53 milliseconds, and this one is {time}")));
  }
  if time > wall.saturating_add(AHEAD) {
    return Err(too_early(format!("An op's time is {time}, more than five minutes ahead of this device's clock at {wall}; offer it again later")));
  }
  Ok(())
}

// A refusal that holds only for now: the same request may succeed later.
fn too_early(message: String) -> ServiceError {
  ServiceError::new(425, "TOO_EARLY", message)
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

  #[test]
  fn a_time_given_after_observing_a_received_op_sorts_after_it() {
    let cases = [
      ("a received time ahead of the wall clock", 100, Some(500), 501),
      ("a received time behind the wall clock", 100, Some(50), 100),
      ("nothing received", 100, None, 100),
    ];
    for (name, wall, received, expected) in cases {
      let clock = Clock::default();
      if let Some(time) = received {
        clock.observe(time);
      }
      assert_eq!(clock.next(wall), expected, "{name}");
      assert_eq!(clock.next(wall), expected + 1, "{name}: then one past it");
    }
  }

  #[test]
  fn a_received_time_is_admitted_unless_no_device_could_make_it_or_it_is_too_far_ahead() {
    let wall = 1_000_000;
    let cases = [
      ("long ago", 10, Ok(())),
      ("now", wall, Ok(())),
      ("as far ahead as clocks drift", wall + AHEAD, Ok(())),
      ("further ahead, for now", wall + AHEAD + 1, Err(425)),
      ("the first time no op may have", NEVER, Err(400)),
      ("past what SQLite stores", u64::MAX, Err(400)),
    ];
    for (name, time, expected) in cases {
      assert_eq!(admit(time, wall).map_err(|error| error.status), expected, "{name}");
    }
  }
}
