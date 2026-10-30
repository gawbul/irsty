//! A token bucket for outgoing messages, so the server does not disconnect us for
//! flooding.

use std::time::Duration;

use tokio::time::Instant;

/// Allows a burst of `capacity` messages, then one every `interval`.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    capacity: u32,
    interval: Duration,
    /// Time at which the bucket will be full again; never earlier than `now -
    /// capacity * interval` in effect.
    full_at: Instant,
}

impl TokenBucket {
    /// Creates a full bucket.
    pub fn new(capacity: u32, interval: Duration, now: Instant) -> Self {
        Self {
            capacity: capacity.max(1),
            interval,
            full_at: now,
        }
    }

    /// The time from which a message may be sent.
    pub fn ready_at(&self) -> Instant {
        // One token is available once the bucket is no more than capacity - 1
        // intervals from full.
        let headroom = self.interval * (self.capacity - 1);
        self.full_at.checked_sub(headroom).unwrap_or(self.full_at)
    }

    /// Takes a token if one is available at `now`.
    pub fn try_take(&mut self, now: Instant) -> bool {
        if now < self.ready_at() {
            return false;
        }
        self.full_at = self.full_at.max(now) + self.interval;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_steady_rate() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut bucket = TokenBucket::new(3, second, start);
        assert!(bucket.try_take(start));
        assert!(bucket.try_take(start));
        assert!(bucket.try_take(start));
        assert!(!bucket.try_take(start));
        assert_eq!(bucket.ready_at(), start + second);
        assert!(bucket.try_take(start + second));
        assert!(!bucket.try_take(start + second));
        // After a long idle period the burst is available again, but no more.
        let later = start + Duration::from_secs(60);
        assert!((0..3).all(|_| bucket.try_take(later)));
        assert!(!bucket.try_take(later));
    }
}
