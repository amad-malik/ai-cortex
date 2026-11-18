//! `Timestamp` (UTC, microsecond precision) and the `Clock` every ID and record time comes from.
//! (CONTRACTS §2.)

/// UTC time with microsecond precision, as microseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_unix_micros(micros: i64) -> Self {
        Self(micros)
    }
    pub const fn unix_micros(self) -> i64 {
        self.0
    }
    /// Whole milliseconds since the epoch (rounded down).
    pub const fn unix_millis(self) -> i64 {
        self.0.div_euclid(1000)
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is before 1970");
        Timestamp(i64::try_from(d.as_micros()).expect("the system clock is past year 294247"))
    }
}

/// An injected clock for tests; compiled only with the `testkit` feature (F1-16).
#[cfg(feature = "testkit")]
pub struct TestClock(std::sync::Mutex<Timestamp>);

#[cfg(feature = "testkit")]
impl TestClock {
    pub fn at(t: Timestamp) -> Self {
        Self(std::sync::Mutex::new(t))
    }
    pub fn set(&self, t: Timestamp) {
        *self.0.lock().expect("TestClock poisoned") = t;
    }
}

#[cfg(feature = "testkit")]
impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().expect("TestClock poisoned")
    }
}
