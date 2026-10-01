//! In-memory token buckets for sign-in and sign-up: a burst of `capacity` attempts, refilled
//! evenly over a minute. Every attempt costs a token, successful or not, so guessing gets nowhere
//! fast and nobody can be locked out of their account by someone else (it's rate limiting, not
//! lockout). One engine instance, so memory is enough.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(60);
/// Past this many keys, full (idle) buckets are dropped so the map can't grow without bound.
const PRUNE_AT: usize = 10_000;

pub struct RateLimiter {
    capacity: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl RateLimiter {
    /// `per_minute` attempts per key per minute.
    pub fn per_minute(per_minute: u32) -> Self {
        Self { capacity: per_minute.into(), buckets: Mutex::default() }
    }

    /// Takes a token for `key`. `Err` holds how long until the next one.
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let rate = self.capacity / WINDOW.as_secs_f64();
        let mut buckets = self.buckets.lock().unwrap();
        if buckets.len() >= PRUNE_AT {
            let capacity = self.capacity;
            buckets.retain(|_, b| b.tokens + now.duration_since(b.updated).as_secs_f64() * rate < capacity);
        }
        let bucket = buckets.entry(key.to_owned()).or_insert(Bucket { tokens: self.capacity, updated: now });
        bucket.tokens = (bucket.tokens + now.duration_since(bucket.updated).as_secs_f64() * rate).min(self.capacity);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / rate).max(Duration::from_secs(1)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_a_burst_then_refills_over_the_window() {
        let limiter = RateLimiter::per_minute(5);
        let start = Instant::now();
        for _ in 0..5 {
            assert!(limiter.check_at("a@x", start).is_ok());
        }
        let wait = limiter.check_at("a@x", start).unwrap_err();
        assert!((11..=12).contains(&wait.as_secs()), "one token per 12 s: {wait:?}");
        assert!(limiter.check_at("b@x", start).is_ok(), "keys are independent");
        assert!(limiter.check_at("a@x", start + Duration::from_secs(12)).is_ok(), "refilled");
    }
}
