//! The injected clock. Code that depends on time takes a [`Clock`] instead of
//! reading the system time, so tests can control it (engineering standards,
//! Rust).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A point in time: milliseconds since the Unix epoch, in UTC.
///
/// This is the representation that Mannie stores (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcMillis(i64);

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

    use super::{Clock, SystemClock, UtcMillis};

    #[test]
    fn system_clock_reads_a_time_after_2026() {
        let year_2026 = UtcMillis::from_millis(1_767_225_600_000);

        assert!(SystemClock.now() > year_2026);
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
