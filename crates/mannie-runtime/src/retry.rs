//! Retries of transient failures (docs/architecture/agent-runtime.md, model
//! calls): exponential backoff with full jitter, and at least as long as the
//! provider's `Retry-After`.

use std::time::Duration;

/// How transient failures are retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The most attempts of one model call, the first one included.
    pub max_attempts: u32,
    /// The delay before the second attempt, before jitter.
    pub base: Duration,
    /// The longest delay, before jitter.
    pub cap: Duration,
    /// The longest `Retry-After` that Mannie waits for. A provider that asks
    /// for longer ends the reply with the time of the next attempt.
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 6,
            base: Duration::from_secs(1),
            cap: Duration::from_secs(60),
            max_retry_after: Duration::from_secs(600),
        }
    }
}

/// What to do after a transient failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    /// Try again after the delay.
    After(Duration),
    /// Stop retrying: the attempts are used up, or the provider asked for
    /// too long a wait.
    GiveUp,
}

impl RetryPolicy {
    /// The decision after attempt `attempt` (from 1) failed. `random` is a
    /// number in [0, 1) for the jitter.
    pub(crate) fn decide(
        &self,
        attempt: u32,
        retry_after: Option<Duration>,
        random: f64,
    ) -> RetryDecision {
        if attempt >= self.max_attempts {
            return RetryDecision::GiveUp;
        }
        if retry_after.is_some_and(|wait| wait > self.max_retry_after) {
            return RetryDecision::GiveUp;
        }
        let exponent = attempt.saturating_sub(1).min(16);
        let ceiling = self.base.saturating_mul(1 << exponent).min(self.cap);
        // Full jitter spreads retries of many clients evenly over the window;
        // half of the base stays as the floor, so a retry never comes at once.
        let jittered = ceiling.mul_f64(random.clamp(0.0, 1.0)).max(self.base / 2);
        let delay = retry_after.map_or(jittered, |wait| wait.max(jittered));
        RetryDecision::After(delay)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{RetryDecision, RetryPolicy};

    #[test]
    fn delays_grow_and_stay_within_the_cap() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.decide(1, None, 0.999),
            RetryDecision::After(Duration::from_secs(1).mul_f64(0.999))
        );
        assert_eq!(
            policy.decide(3, None, 0.5),
            RetryDecision::After(Duration::from_secs(2))
        );
        assert_eq!(
            policy.decide(5, None, 0.999),
            RetryDecision::After(Duration::from_secs(16).mul_f64(0.999))
        );
        assert_eq!(
            policy.decide(2, None, 0.0),
            RetryDecision::After(Duration::from_millis(500))
        );
    }

    #[test]
    fn retry_after_is_honored_and_a_long_one_gives_up() {
        let policy = RetryPolicy::default();

        assert_eq!(
            policy.decide(1, Some(Duration::from_secs(20)), 0.5),
            RetryDecision::After(Duration::from_secs(20))
        );
        assert_eq!(
            policy.decide(1, Some(Duration::from_secs(601)), 0.5),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn the_last_attempt_gives_up() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.decide(6, None, 0.5), RetryDecision::GiveUp);
        assert!(matches!(
            policy.decide(5, None, 0.5),
            RetryDecision::After(_)
        ));
    }
}
