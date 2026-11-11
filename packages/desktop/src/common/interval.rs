use std::{
  num::NonZeroU64,
  time::{Duration, Instant},
};

use serde::{Deserialize, Deserializer};

pub fn deserialize_refresh_interval<'de, D>(
  deserializer: D,
) -> Result<u64, D::Error>
where
  D: Deserializer<'de>,
{
  NonZeroU64::deserialize(deserializer).map(NonZeroU64::get)
}

/// An interval timer for synchronous contexts using crossbeam.
///
/// For use with crossbeam's `select!` macro.
pub struct SyncInterval {
  ticks: crossbeam::channel::Receiver<Instant>,
  is_first: bool,
}

impl SyncInterval {
  pub fn new(interval_ms: u64) -> Self {
    assert!(interval_ms > 0, "Refresh interval must be positive.");
    Self {
      ticks: crossbeam::channel::tick(Duration::from_millis(interval_ms)),
      is_first: true,
    }
  }

  /// Returns a receiver that will get a message at the next tick time.
  pub fn tick(&mut self) -> crossbeam::channel::Receiver<Instant> {
    if self.is_first {
      // Emit immediately on the first tick.
      self.is_first = false;
      crossbeam::channel::after(Duration::from_secs(0))
    } else {
      self.ticks.clone()
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn first_tick_is_immediate_and_later_ticks_wait() {
    let mut interval = SyncInterval::new(60_000);
    interval
      .tick()
      .recv_timeout(Duration::from_secs(1))
      .unwrap();
    assert_eq!(
      interval.tick().try_recv(),
      Err(crossbeam::channel::TryRecvError::Empty)
    );
  }

  #[test]
  #[should_panic(expected = "Refresh interval must be positive.")]
  fn zero_interval_cannot_spin_forever() {
    SyncInterval::new(0);
  }
}

/// An interval timer for asynchronous contexts using tokio.
pub struct AsyncInterval {
  interval: tokio::time::Interval,
}

impl AsyncInterval {
  pub fn new(interval_ms: u64) -> Self {
    let mut interval =
      tokio::time::interval(Duration::from_millis(interval_ms));

    // Skip missed ticks when the interval runs. This prevents a burst
    // of backlogged ticks after a delay.
    interval
      .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    Self { interval }
  }

  /// Returns a future that will complete at the next tick time.
  pub async fn tick(&mut self) {
    self.interval.tick().await;
  }
}
