//! WASI as a service sees it: closed. No environment, arguments, files,
//! sockets or standard input, and its output goes nowhere. A call that
//! writes sees the op's time on every clock and draws its randomness from the
//! op's hash, so every replica's run sees the same; a GET, which no replica
//! repeats, sees this device's clocks and randomness.

use std::time::Duration;

use rand::SeedableRng;
use rand::rngs::StdRng;
use wasmtime_wasi::{HostMonotonicClock, HostWallClock, WasiCtx};

// A clock stopped at an op's time.
struct Stopped(Duration);

impl HostWallClock for Stopped {
  fn resolution(&self) -> Duration {
    Duration::from_millis(1)
  }

  fn now(&self) -> Duration {
    self.0
  }
}

impl HostMonotonicClock for Stopped {
  fn resolution(&self) -> u64 {
    1_000_000
  }

  fn now(&self) -> u64 {
    self.0.as_nanos() as u64
  }
}

// WASI for an instance; `op` is the hash and time of the op a write runs in.
pub(super) fn closed(op: Option<(&str, u64)>) -> WasiCtx {
  let mut builder = WasiCtx::builder();
  if let Some((hash, time)) = op {
    let seed = seed(hash);
    builder
      .wall_clock(Stopped(Duration::from_millis(time)))
      .monotonic_clock(Stopped(Duration::from_millis(time)))
      .secure_random(StdRng::from_seed(seed))
      .insecure_random(StdRng::from_seed(seed))
      .insecure_random_seed(u128::from_le_bytes(seed[..16].try_into().expect("a seed holds 16 bytes")));
  }
  builder.build()
}

// 32 bytes from an op's hex hash.
fn seed(hash: &str) -> [u8; 32] {
  let mut seed = [0u8; 32];
  for (index, byte) in seed.iter_mut().enumerate() {
    *byte = hash.get(index * 2..index * 2 + 2).and_then(|hex| u8::from_str_radix(hex, 16).ok()).unwrap_or(0);
  }
  seed
}
