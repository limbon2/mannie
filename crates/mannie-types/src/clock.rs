//! The injected clock. Code that depends on time takes a [`Clock`] instead of
//! reading the system time, so tests can control it (engineering standards,
//! Rust).

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A point in time: milliseconds since the Unix epoch, in UTC.
///
/// This is the representation that Mannie stores (ADR-0009) and sends over
/// IPC, where it is a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(i64);

impl specta::Type for UtcMillis {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        // specta refuses 64-bit integers, because JavaScript numbers lose
        // precision above 2^53. Times in milliseconds stay far below that
        // until the year 287396, so the bindings type them as a plain
        // `number`, like the `u32` whose definition this borrows.
        <u32 as specta::Type>::definition(types)
    }
}

impl UtcMillis {
    /// The Unix epoch.
    pub const EPOCH: Self = Self(0);

    /// A point in time from milliseconds since the Unix epoch.
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the Unix epoch.
    pub const fn as_millis(self) -> i64 {
        self.0
    }

    /// This point in time moved forward by `duration`, saturating at the
    /// largest representable value.
    #[must_use]
    pub fn saturating_add(self, duration: Duration) -> Self {
        let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self(self.0.saturating_add(millis))
    }
}

impl fmt::Display for UtcMillis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ms after the Unix epoch", self.0)
    }
}

/// A source of the current time.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current time in UTC.
    fn now(&self) -> UtcMillis;
}

/// A clock that tests and simulations set and move by hand.
#[derive(Debug)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    /// A clock that reads `start` until it is moved.
    pub fn new(start: UtcMillis) -> Self {
        Self(AtomicI64::new(start.as_millis()))
    }

    /// Moves the clock forward by `duration`.
    pub fn advance(&self, duration: Duration) {
        let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        self.0.fetch_add(millis, Ordering::SeqCst);
    }

    /// Sets the clock to `now`.
    pub fn set(&self, now: UtcMillis) {
        self.0.store(now.as_millis(), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
}

/// The operating system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "SystemClock is the one place that reads the system time; everything else takes a Clock"
    )]
    fn now(&self) -> UtcMillis {
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since_epoch) => i64::try_from(since_epoch.as_millis()).unwrap_or(i64::MAX),
            // A clock set before 1970 is reported as negative milliseconds.
            Err(before_epoch) => i64::try_from(before_epoch.duration().as_millis())
                .map_or(i64::MIN, |millis| -millis),
        };
        UtcMillis(millis)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Clock, ManualClock, SystemClock, UtcMillis};

    #[test]
    fn system_clock_reads_a_time_after_2026() {
        let year_2026 = UtcMillis::from_millis(1_767_225_600_000);

        assert!(SystemClock.now() > year_2026);
    }

    #[test]
    fn a_manual_clock_reads_what_it_was_set_to() {
        let clock = ManualClock::new(UtcMillis::from_millis(1_000));

        clock.advance(Duration::from_millis(500));
        let advanced = clock.now();
        clock.set(UtcMillis::from_millis(42));

        assert_eq!(advanced, UtcMillis::from_millis(1_500));
        assert_eq!(clock.now(), UtcMillis::from_millis(42));
    }

    #[test]
    fn serializes_as_a_number() {
        let time = UtcMillis::from_millis(1_791_892_800_000);

        assert_eq!(serde_json::to_string(&time).unwrap(), "1791892800000");
        assert_eq!(
            serde_json::from_str::<UtcMillis>("1791892800000").unwrap(),
            time
        );
    }

    #[test]
    fn saturating_add_moves_forward_and_saturates() {
        let start = UtcMillis::from_millis(1_000);

        assert_eq!(
            start.saturating_add(Duration::from_millis(250)).as_millis(),
            1_250
        );
        assert_eq!(
            UtcMillis::from_millis(i64::MAX - 1).saturating_add(Duration::from_secs(1)),
            UtcMillis::from_millis(i64::MAX)
        );
    }
}
