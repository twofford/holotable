//! A simple in-memory rate limiter for sign-in attempts (spec ACC-007).
//!
//! It counts attempts per key (an IP address or an email) in a fixed window.
//! It forgets everything on restart, and it counts per server process, which is
//! fine for one small server.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct RateLimiter {
    max_attempts: u32,
    window: Duration,
    /// key -> (attempts so far, when the window started).
    ///
    /// A plain `std::sync::Mutex` is right here: the lock is held only for a
    /// quick map update and never across an `.await`. (Holding a std mutex
    /// across an await could block a tokio worker thread.)
    attempts: Mutex<HashMap<String, (u32, Instant)>>,
}

/// Clean out stale keys once the map gets this big, so it can't grow forever.
const PRUNE_AT: usize = 10_000;

impl RateLimiter {
    pub fn new(max_attempts: u32, window: Duration) -> Self {
        Self {
            max_attempts,
            window,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// Records an attempt for `key`. Returns `false` if the key is over its limit.
    pub fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        // `lock()` fails only if another thread panicked while holding the lock.
        // The map is still usable then, so take it anyway with `into_inner`.
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());

        if map.len() >= PRUNE_AT {
            let window = self.window;
            map.retain(|_, (_, start)| now.duration_since(*start) < window);
        }

        // `entry` finds the key or inserts a fresh counter, in one lookup.
        let (count, start) = map.entry(key.to_owned()).or_insert((0, now));
        if now.duration_since(*start) >= self.window {
            *count = 0;
            *start = now;
        }
        *count += 1;
        *count <= self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_the_limit_and_keys_are_separate() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.allow("a"));
        assert!(limiter.allow("a"));
        assert!(limiter.allow("a"));
        assert!(!limiter.allow("a"));
        assert!(limiter.allow("b"));
    }

    #[test]
    fn window_resets() {
        let limiter = RateLimiter::new(1, Duration::from_millis(20));
        assert!(limiter.allow("a"));
        assert!(!limiter.allow("a"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(limiter.allow("a"));
    }
}
