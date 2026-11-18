//! Retry on 429 `busy` (R-9): bounded attempts, bounded waits, an injectable sleeper (W1-43 Q-10).

use std::time::Duration;

/// Waits. Production sleeps the thread; a test records the waits and does not sleep.
pub trait Sleeper: Send + Sync {
    fn sleep(&self, wait: Duration);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, wait: Duration) {
        std::thread::sleep(wait);
    }
}

/// 4 attempts, a wait of `Retry-After` seconds capped at 5, the sum of the waits capped at 10 (W1-43 Q-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub max_wait_each: Duration,
    pub max_total_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 4,
            max_wait_each: Duration::from_secs(5),
            max_total_wait: Duration::from_secs(10),
        }
    }
}

impl RetryPolicy {
    /// The wait for one `Retry-After` value: whole seconds, at least 1, at most `max_wait_each`; anything that is
    /// not a plain non-negative decimal integer (a date, a sign, a fraction, empty, too large) is 1 second (RT-02).
    pub fn wait_for(&self, header: Option<&str>) -> Duration {
        let one = Duration::from_secs(1);
        let secs = match header {
            Some(h) if !h.is_empty() && h.bytes().all(|b| b.is_ascii_digit()) => {
                h.parse::<u64>().ok()
            }
            _ => None,
        };
        match secs {
            Some(s) => Duration::from_secs(s.max(1)).min(self.max_wait_each),
            None => one.min(self.max_wait_each),
        }
    }
}
