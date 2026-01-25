//! Auto-retry logic for network operations.

use std::time::Duration;

/// Retry policy configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts.
    pub max_retries: usize,
    /// Initial delay between retries.
    pub initial_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Multiplier for exponential backoff.
    pub backoff_multiplier: f64,
    /// Whether to add jitter to delays.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Create a new retry policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a policy with no retries.
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Create a policy for network operations.
    pub fn network() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }

    /// Set maximum retries.
    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set initial delay.
    pub fn with_initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Set maximum delay.
    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Calculate delay for a given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let base_delay =
            self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi((attempt - 1) as i32);
        let capped_delay = base_delay.min(self.max_delay.as_secs_f64());

        let delay = if self.jitter {
            // Add up to 25% jitter
            let jitter = (rand_simple() * 0.25) * capped_delay;
            capped_delay + jitter
        } else {
            capped_delay
        };

        Duration::from_secs_f64(delay)
    }

    /// Check if retries are enabled.
    pub fn retries_enabled(&self) -> bool {
        self.max_retries > 0
    }
}

/// State tracker for retry attempts.
#[derive(Debug)]
pub struct RetryState {
    /// The retry policy.
    policy: RetryPolicy,
    /// Current attempt number (0-indexed).
    attempt: usize,
    /// Last error encountered.
    last_error: Option<String>,
}

impl RetryState {
    /// Create a new retry state with the given policy.
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            policy,
            attempt: 0,
            last_error: None,
        }
    }

    /// Create a retry state with default policy.
    pub fn with_default_policy() -> Self {
        Self::new(RetryPolicy::default())
    }

    /// Get the current attempt number (0-indexed).
    pub fn attempt(&self) -> usize {
        self.attempt
    }

    /// Get the last error.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Check if more retries are available.
    pub fn can_retry(&self) -> bool {
        self.attempt < self.policy.max_retries
    }

    /// Record a failure and prepare for retry.
    ///
    /// Returns `true` if a retry is available, `false` if exhausted.
    pub fn record_failure(&mut self, error: impl Into<String>) -> bool {
        self.last_error = Some(error.into());
        self.attempt += 1;
        self.can_retry()
    }

    /// Get the delay before the next retry attempt.
    pub fn next_delay(&self) -> Duration {
        self.policy.delay_for_attempt(self.attempt)
    }

    /// Reset the retry state.
    pub fn reset(&mut self) {
        self.attempt = 0;
        self.last_error = None;
    }

    /// Get the policy.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }
}

/// Errors that are typically retryable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryableErrorKind {
    /// Network connection error.
    ConnectionError,
    /// Timeout.
    Timeout,
    /// Temporary unavailability.
    TemporaryUnavailable,
    /// Rate limited.
    RateLimited,
}

/// Check if an error is retryable.
pub fn is_retryable_error(error: &str) -> bool {
    let error_lower = error.to_lowercase();

    // Connection errors
    if error_lower.contains("connection refused")
        || error_lower.contains("connection reset")
        || error_lower.contains("broken pipe")
        || error_lower.contains("network unreachable")
        || error_lower.contains("host unreachable")
    {
        return true;
    }

    // Timeout errors
    if error_lower.contains("timed out")
        || error_lower.contains("timeout")
        || error_lower.contains("deadline exceeded")
    {
        return true;
    }

    // Temporary errors
    if error_lower.contains("try again")
        || error_lower.contains("temporary")
        || error_lower.contains("service unavailable")
    {
        return true;
    }

    false
}

/// Simple pseudo-random number generator (0.0 - 1.0).
/// Used for jitter without requiring a separate random crate.
fn rand_simple() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_u64(std::time::Instant::now().elapsed().as_nanos() as u64);
    (hasher.finish() as f64) / (u64::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries, 3);
        assert!(policy.retries_enabled());
    }

    #[test]
    fn test_no_retry_policy() {
        let policy = RetryPolicy::no_retry();
        assert_eq!(policy.max_retries, 0);
        assert!(!policy.retries_enabled());
    }

    #[test]
    fn test_retry_state() {
        let policy = RetryPolicy::new().with_max_retries(2);
        let mut state = RetryState::new(policy);

        assert!(state.can_retry());
        assert!(state.record_failure("error 1"));
        assert!(state.can_retry());
        assert!(!state.record_failure("error 2"));
        assert!(!state.can_retry());
    }

    #[test]
    fn test_exponential_backoff() {
        let policy = RetryPolicy::new()
            .with_initial_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(10))
            .with_max_retries(5);

        // First attempt has no delay
        assert_eq!(policy.delay_for_attempt(0), Duration::ZERO);

        // Subsequent attempts have increasing delays (with potential jitter)
        let delay1 = policy.delay_for_attempt(1);
        let delay2 = policy.delay_for_attempt(2);
        let delay3 = policy.delay_for_attempt(3);

        // Delays should increase (accounting for jitter)
        assert!(delay1.as_secs_f64() >= 0.75); // At least initial_delay - jitter
        assert!(delay2.as_secs_f64() >= 1.5); // At least 2x
        assert!(delay3.as_secs_f64() >= 3.0); // At least 4x
    }

    #[test]
    fn test_retryable_errors() {
        assert!(is_retryable_error("Connection refused"));
        assert!(is_retryable_error("Operation timed out"));
        assert!(is_retryable_error("service unavailable"));
        assert!(is_retryable_error("try again later"));
        assert!(!is_retryable_error("Permission denied"));
        assert!(!is_retryable_error("File not found"));
    }
}
