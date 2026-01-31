//! Error Recovery System
//!
//! Implements robust error handling with retry strategies:
//! - Exponential backoff
//! - Circuit breaker pattern
//! - Fallback strategies
//! - Error classification
//!
//! Industry standard: Netflix Hystrix, resilience4j

use anyhow::Result;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Classification of errors for recovery strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Temporary failure, retry likely to succeed
    Transient,
    /// Rate limiting, need backoff
    RateLimited,
    /// Resource unavailable, may recover
    ResourceUnavailable,
    /// Invalid input, retry won't help
    ValidationError,
    /// Authorization failed, needs intervention
    AuthError,
    /// System error, may need circuit break
    SystemError,
    /// Unknown error type
    Unknown,
}

impl ErrorClass {
    /// Classify an error from its message
    pub fn from_error(error: &str) -> Self {
        let lower = error.to_lowercase();

        if lower.contains("rate limit") || lower.contains("too many requests") || lower.contains("429") {
            Self::RateLimited
        } else if lower.contains("timeout") || lower.contains("connection") || lower.contains("temporary") {
            Self::Transient
        } else if lower.contains("not found") || lower.contains("unavailable") || lower.contains("503") {
            Self::ResourceUnavailable
        } else if lower.contains("invalid") || lower.contains("validation") || lower.contains("400") {
            Self::ValidationError
        } else if lower.contains("unauthorized") || lower.contains("forbidden") || lower.contains("401") || lower.contains("403") {
            Self::AuthError
        } else if lower.contains("internal") || lower.contains("500") || lower.contains("panic") {
            Self::SystemError
        } else {
            Self::Unknown
        }
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient | Self::RateLimited | Self::ResourceUnavailable | Self::Unknown)
    }
}

/// Retry policy configuration
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum retry attempts
    pub max_retries: usize,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
    /// Add jitter to prevent thundering herd
    pub add_jitter: bool,
    /// Jitter factor (0.0 - 1.0)
    pub jitter_factor: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            add_jitter: true,
            jitter_factor: 0.2,
        }
    }
}

impl RetryPolicy {
    /// Create an aggressive retry policy (many fast retries)
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            add_jitter: true,
            jitter_factor: 0.1,
        }
    }

    /// Create a conservative retry policy (few slow retries)
    pub fn conservative() -> Self {
        Self {
            max_retries: 2,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 3.0,
            add_jitter: true,
            jitter_factor: 0.3,
        }
    }

    /// Create a rate-limit aware policy
    pub fn rate_limit_aware() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(120),
            backoff_multiplier: 2.5,
            add_jitter: true,
            jitter_factor: 0.2,
        }
    }

    /// Calculate delay for a given attempt
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        let base = self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
        let capped = base.min(self.max_delay.as_secs_f64());

        let delay = if self.add_jitter {
            let jitter = capped * self.jitter_factor * (rand_simple() * 2.0 - 1.0);
            (capped + jitter).max(0.0)
        } else {
            capped
        };

        Duration::from_secs_f64(delay)
    }
}

/// Simple pseudo-random for jitter (avoid heavy rand dependency)
fn rand_simple() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64);
    (hasher.finish() as f64) / (u64::MAX as f64)
}

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation
    Closed,
    /// Allowing some requests through to test
    HalfOpen,
    /// Blocking all requests
    Open,
}

/// Circuit breaker for preventing cascade failures
pub struct CircuitBreaker {
    name: String,
    state: Arc<RwLock<CircuitState>>,
    failure_count: AtomicUsize,
    success_count: AtomicUsize,
    last_failure: Arc<RwLock<Option<Instant>>>,
    config: CircuitBreakerConfig,
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Failures before opening circuit
    pub failure_threshold: usize,
    /// Successes needed to close from half-open
    pub success_threshold: usize,
    /// Time to wait before half-opening
    pub open_duration: Duration,
    /// Time window for counting failures
    pub failure_window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            open_duration: Duration::from_secs(30),
            failure_window: Duration::from_secs(60),
        }
    }
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(name: &str) -> Self {
        Self::with_config(name, CircuitBreakerConfig::default())
    }

    /// Create with custom config
    pub fn with_config(name: &str, config: CircuitBreakerConfig) -> Self {
        Self {
            name: name.to_string(),
            state: Arc::new(RwLock::new(CircuitState::Closed)),
            failure_count: AtomicUsize::new(0),
            success_count: AtomicUsize::new(0),
            last_failure: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Get current state
    pub async fn state(&self) -> CircuitState {
        let mut state = self.state.write().await;

        // Check if we should transition from open to half-open
        if *state == CircuitState::Open {
            if let Some(last) = *self.last_failure.read().await {
                if last.elapsed() >= self.config.open_duration {
                    *state = CircuitState::HalfOpen;
                    self.success_count.store(0, Ordering::SeqCst);
                    info!("Circuit breaker '{}' transitioning to half-open", self.name);
                }
            }
        }

        *state
    }

    /// Check if requests should be allowed
    pub async fn allow(&self) -> bool {
        match self.state().await {
            CircuitState::Closed => true,
            CircuitState::HalfOpen => true, // Allow test request
            CircuitState::Open => false,
        }
    }

    /// Record a successful operation
    pub async fn record_success(&self) {
        let mut state = self.state.write().await;

        match *state {
            CircuitState::HalfOpen => {
                let count = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= self.config.success_threshold {
                    *state = CircuitState::Closed;
                    self.failure_count.store(0, Ordering::SeqCst);
                    info!("Circuit breaker '{}' closed after recovery", self.name);
                }
            }
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);
            }
            _ => {}
        }
    }

    /// Record a failed operation
    pub async fn record_failure(&self) {
        *self.last_failure.write().await = Some(Instant::now());

        let mut state = self.state.write().await;

        match *state {
            CircuitState::Closed => {
                let count = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if count >= self.config.failure_threshold {
                    *state = CircuitState::Open;
                    warn!("Circuit breaker '{}' opened after {} failures", self.name, count);
                }
            }
            CircuitState::HalfOpen => {
                // Immediate open on failure during half-open
                *state = CircuitState::Open;
                self.success_count.store(0, Ordering::SeqCst);
                warn!("Circuit breaker '{}' reopened after half-open failure", self.name);
            }
            _ => {}
        }
    }

    /// Reset the circuit breaker
    pub async fn reset(&self) {
        *self.state.write().await = CircuitState::Closed;
        self.failure_count.store(0, Ordering::SeqCst);
        self.success_count.store(0, Ordering::SeqCst);
        *self.last_failure.write().await = None;
        info!("Circuit breaker '{}' reset", self.name);
    }
}

/// Action to take for recovery
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    /// Retry the operation
    Retry { attempt: usize, delay: Duration },
    /// Use fallback value/behavior
    Fallback(String),
    /// Fail immediately
    Fail(String),
    /// Circuit is open, wait and retry
    CircuitOpen { retry_after: Duration },
}

/// Recovery strategy combining policies
pub struct RecoveryStrategy {
    name: String,
    retry_policy: RetryPolicy,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    fallback: Option<String>,
    metrics: RecoveryMetrics,
}

/// Metrics for recovery operations
struct RecoveryMetrics {
    total_attempts: AtomicU64,
    successful: AtomicU64,
    retried: AtomicU64,
    failed: AtomicU64,
    circuit_opened: AtomicU64,
    fallbacks_used: AtomicU64,
}

impl RecoveryMetrics {
    fn new() -> Self {
        Self {
            total_attempts: AtomicU64::new(0),
            successful: AtomicU64::new(0),
            retried: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            circuit_opened: AtomicU64::new(0),
            fallbacks_used: AtomicU64::new(0),
        }
    }
}

impl RecoveryStrategy {
    /// Create a new recovery strategy
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            retry_policy: RetryPolicy::default(),
            circuit_breaker: None,
            fallback: None,
            metrics: RecoveryMetrics::new(),
        }
    }

    /// Set retry policy
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Add circuit breaker
    pub fn with_circuit_breaker(mut self, breaker: Arc<CircuitBreaker>) -> Self {
        self.circuit_breaker = Some(breaker);
        self
    }

    /// Set fallback value
    pub fn with_fallback(mut self, fallback: &str) -> Self {
        self.fallback = Some(fallback.to_string());
        self
    }

    /// Execute with recovery
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.metrics.total_attempts.fetch_add(1, Ordering::Relaxed);

        // Check circuit breaker
        if let Some(ref cb) = self.circuit_breaker {
            if !cb.allow().await {
                self.metrics.circuit_opened.fetch_add(1, Ordering::Relaxed);
                return Err(anyhow::anyhow!("Circuit breaker open"));
            }
        }

        let mut attempt = 0;
        let mut last_error: Option<anyhow::Error> = None;

        loop {
            match operation().await {
                Ok(result) => {
                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_success().await;
                    }
                    self.metrics.successful.fetch_add(1, Ordering::Relaxed);
                    return Ok(result);
                }
                Err(e) => {
                    let error_class = ErrorClass::from_error(&e.to_string());

                    if let Some(ref cb) = self.circuit_breaker {
                        cb.record_failure().await;
                    }

                    if !error_class.is_retryable() || attempt >= self.retry_policy.max_retries {
                        self.metrics.failed.fetch_add(1, Ordering::Relaxed);
                        last_error = Some(e);
                        break;
                    }

                    let delay = self.retry_policy.delay_for_attempt(attempt);
                    debug!(
                        "Retry {} for '{}' after {:?} (error: {})",
                        attempt + 1,
                        self.name,
                        delay,
                        e
                    );

                    self.metrics.retried.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
    }

    /// Get recovery stats
    pub fn stats(&self) -> RecoveryStats {
        RecoveryStats {
            name: self.name.clone(),
            total_attempts: self.metrics.total_attempts.load(Ordering::Relaxed),
            successful: self.metrics.successful.load(Ordering::Relaxed),
            retried: self.metrics.retried.load(Ordering::Relaxed),
            failed: self.metrics.failed.load(Ordering::Relaxed),
            circuit_opened: self.metrics.circuit_opened.load(Ordering::Relaxed),
            fallbacks_used: self.metrics.fallbacks_used.load(Ordering::Relaxed),
        }
    }
}

/// Statistics for recovery operations
#[derive(Debug, Clone)]
pub struct RecoveryStats {
    pub name: String,
    pub total_attempts: u64,
    pub successful: u64,
    pub retried: u64,
    pub failed: u64,
    pub circuit_opened: u64,
    pub fallbacks_used: u64,
}

impl RecoveryStats {
    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_attempts == 0 {
            1.0
        } else {
            self.successful as f64 / self.total_attempts as f64
        }
    }

    /// Format for display
    pub fn format(&self) -> String {
        format!(
            "{}: {:.1}% success ({}/{} attempts, {} retries, {} failed)",
            self.name,
            self.success_rate() * 100.0,
            self.successful,
            self.total_attempts,
            self.retried,
            self.failed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        assert_eq!(ErrorClass::from_error("rate limit exceeded"), ErrorClass::RateLimited);
        assert_eq!(ErrorClass::from_error("connection timeout"), ErrorClass::Transient);
        assert_eq!(ErrorClass::from_error("invalid input"), ErrorClass::ValidationError);
        assert_eq!(ErrorClass::from_error("unauthorized"), ErrorClass::AuthError);
        assert_eq!(ErrorClass::from_error("internal server error"), ErrorClass::SystemError);
        assert_eq!(ErrorClass::from_error("something weird"), ErrorClass::Unknown);
    }

    #[test]
    fn test_retry_policy_delay() {
        let policy = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(10),
            add_jitter: false,
            ..Default::default()
        };

        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
    }

    #[test]
    fn test_retry_policy_max_cap() {
        let policy = RetryPolicy {
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 10.0,
            max_delay: Duration::from_secs(5),
            add_jitter: false,
            ..Default::default()
        };

        // Should be capped at 5 seconds
        assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_circuit_breaker_normal() {
        let cb = CircuitBreaker::new("test");

        assert!(cb.allow().await);
        cb.record_success().await;
        assert_eq!(cb.state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::with_config("test", config);

        cb.record_failure().await;
        cb.record_failure().await;
        assert!(cb.allow().await); // Still closed

        cb.record_failure().await;
        assert_eq!(cb.state().await, CircuitState::Open);
        assert!(!cb.allow().await);
    }

    #[tokio::test]
    async fn test_recovery_strategy_success() {
        let strategy = RecoveryStrategy::new("test");

        let result: Result<i32> = strategy.execute(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);

        let stats = strategy.stats();
        assert_eq!(stats.successful, 1);
        assert_eq!(stats.failed, 0);
    }

    #[tokio::test]
    async fn test_recovery_strategy_retry() {
        let strategy = RecoveryStrategy::new("test")
            .with_retry(RetryPolicy {
                max_retries: 2,
                initial_delay: Duration::from_millis(1),
                add_jitter: false,
                ..Default::default()
            });

        let attempt = Arc::new(AtomicUsize::new(0));
        let attempt_clone = attempt.clone();

        let result: Result<i32> = strategy
            .execute(|| {
                let attempt = attempt_clone.clone();
                async move {
                    let n = attempt.fetch_add(1, Ordering::SeqCst);
                    if n < 2 {
                        Err(anyhow::anyhow!("temporary failure"))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[test]
    fn test_recovery_stats_format() {
        let stats = RecoveryStats {
            name: "api".to_string(),
            total_attempts: 100,
            successful: 95,
            retried: 10,
            failed: 5,
            circuit_opened: 1,
            fallbacks_used: 0,
        };

        let formatted = stats.format();
        assert!(formatted.contains("95.0%"));
        assert!(formatted.contains("95/100"));
    }
}
