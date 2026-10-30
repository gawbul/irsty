//! Reconnect delays: exponential backoff with jitter.

use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

/// Exponential backoff with "equal jitter": each delay is half the exponential
/// step plus a random amount up to the other half. Unlike full jitter this never
/// retries immediately, which matters when a server is rejecting us.
#[derive(Clone, Debug)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    attempt: u32,
    rng: u64,
}

impl Backoff {
    /// Creates a backoff starting at `base` and never exceeding `cap`.
    pub fn new(base: Duration, cap: Duration) -> Self {
        // RandomState is randomly seeded per process; good enough for jitter.
        let seed = RandomState::new().hash_one(0x6972_7374_7921_u64) | 1;
        Self {
            base,
            cap,
            attempt: 0,
            rng: seed,
        }
    }

    /// The next delay.
    pub fn next_delay(&mut self) -> Duration {
        let step = self
            .base
            .saturating_mul(1 << self.attempt.min(20))
            .min(self.cap);
        self.attempt = self.attempt.saturating_add(1);
        let half = step / 2;
        let jitter_ms = u64::try_from(half.as_millis()).unwrap_or(u64::MAX);
        let random = if jitter_ms == 0 {
            0
        } else {
            self.next_random() % (jitter_ms + 1)
        };
        half + Duration::from_millis(random)
    }

    /// Starts again from `base`, after a connection proved stable.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// xorshift64*.
    fn next_random(&mut self) -> u64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_with_jitter_and_caps() {
        let base = Duration::from_secs(1);
        let cap = Duration::from_secs(300);
        let mut backoff = Backoff::new(base, cap);
        let mut step = base;
        for _ in 0..15 {
            let delay = backoff.next_delay();
            assert!(
                delay >= step / 2 && delay <= step,
                "{delay:?} outside {step:?}"
            );
            step = (step * 2).min(cap);
        }
        backoff.reset();
        assert!(backoff.next_delay() <= base);
    }

    #[test]
    fn delays_vary() {
        let mut backoff = Backoff::new(Duration::from_secs(64), Duration::from_secs(64));
        let delays: std::collections::HashSet<_> = (0..20).map(|_| backoff.next_delay()).collect();
        assert!(delays.len() > 1);
    }
}
