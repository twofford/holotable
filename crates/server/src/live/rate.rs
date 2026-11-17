//! Per-connection message rate limit (spec SYN-009).
//!
//! Counts messages in one-second windows. Over the limit, messages are
//! dropped. A connection that goes over in several windows is disconnected.
//! A clean window forgives one earlier strike, so a single burst is harmless.

use std::time::{Duration, Instant};

pub const MAX_PER_SECOND: u32 = 50;
const STRIKES_TO_DISCONNECT: u32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Drop,
    Disconnect,
}

pub struct MessageRate {
    window_start: Instant,
    count: u32,
    strikes: u32,
    /// Whether the current window has already been counted as a strike.
    struck: bool,
}

impl MessageRate {
    pub fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            count: 0,
            strikes: 0,
            struck: false,
        }
    }

    /// Records one message arriving at `now`. Taking `now` as an argument,
    /// rather than reading the clock inside, is what makes this testable.
    pub fn check(&mut self, now: Instant) -> Verdict {
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            if !self.struck {
                self.strikes = self.strikes.saturating_sub(1);
            }
            self.window_start = now;
            self.count = 0;
            self.struck = false;
        }
        self.count += 1;
        if self.count <= MAX_PER_SECOND {
            return Verdict::Allow;
        }
        if !self.struck {
            self.struck = true;
            self.strikes += 1;
        }
        if self.strikes >= STRIKES_TO_DISCONNECT {
            Verdict::Disconnect
        } else {
            Verdict::Drop
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_drops() {
        let t = Instant::now();
        let mut rate = MessageRate::new(t);
        for _ in 0..MAX_PER_SECOND {
            assert_eq!(rate.check(t), Verdict::Allow);
        }
        assert_eq!(rate.check(t), Verdict::Drop);
        // A new second starts fresh.
        assert_eq!(rate.check(t + Duration::from_secs(1)), Verdict::Allow);
    }

    #[test]
    fn repeated_flooding_disconnects() {
        let t = Instant::now();
        let mut rate = MessageRate::new(t);
        let mut last = Verdict::Allow;
        for second in 0..3 {
            let now = t + Duration::from_secs(second);
            for _ in 0..=MAX_PER_SECOND {
                last = rate.check(now);
            }
        }
        assert_eq!(last, Verdict::Disconnect);
    }

    #[test]
    fn clean_seconds_forgive_strikes() {
        let t = Instant::now();
        let mut rate = MessageRate::new(t);
        let flood = |rate: &mut MessageRate, now| {
            for _ in 0..=MAX_PER_SECOND {
                rate.check(now);
            }
        };
        flood(&mut rate, t);
        flood(&mut rate, t + Duration::from_secs(1));
        rate.check(t + Duration::from_secs(2)); // A quiet second.
        rate.check(t + Duration::from_secs(3)); // Another.
        flood(&mut rate, t + Duration::from_secs(4));
        assert_eq!(rate.check(t + Duration::from_secs(4)), Verdict::Drop);
    }
}
