//! Rate Limiting for Channel Endpoints
//!
//! Provides per-user and per-channel rate limiting to prevent abuse.
//!
//! Features:
//! - Per-user request limits
//! - Per-channel global limits
//! - Sliding window algorithm
//! - Burst allowance
//! - Automatic cleanup of expired entries

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Rate limiter configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per window
    pub max_requests: u32,
    /// Time window in seconds
    pub window_secs: u64,
    /// Burst allowance (extra requests allowed in short bursts)
    pub burst_allowance: u32,
    /// Cooldown period after hitting limit (seconds)
    pub cooldown_secs: u64,
    /// Enable global channel limits
    pub enable_global_limit: bool,
    /// Global limit (requests per window across all users)
    pub global_max_requests: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: 30,        // 30 requests
            window_secs: 60,         // per minute
            burst_allowance: 5,      // allow 5 extra in bursts
            cooldown_secs: 30,       // 30 second cooldown after limit
            enable_global_limit: true,
            global_max_requests: 1000, // 1000 total per minute
        }
    }
}

impl RateLimitConfig {
    /// Strict limits for high-traffic channels
    pub fn strict() -> Self {
        Self {
            max_requests: 10,
            window_secs: 60,
            burst_allowance: 2,
            cooldown_secs: 60,
            enable_global_limit: true,
            global_max_requests: 500,
        }
    }

    /// Relaxed limits for trusted users
    pub fn relaxed() -> Self {
        Self {
            max_requests: 100,
            window_secs: 60,
            burst_allowance: 20,
            cooldown_secs: 10,
            enable_global_limit: true,
            global_max_requests: 5000,
        }
    }
}

/// Rate limit entry for tracking requests
#[derive(Debug, Clone)]
struct RateLimitEntry {
    /// Request timestamps in the current window
    requests: Vec<Instant>,
    /// When the user was last rate limited
    last_limited: Option<Instant>,
    /// Whether currently in cooldown
    in_cooldown: bool,
}

impl RateLimitEntry {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
            last_limited: None,
            in_cooldown: false,
        }
    }

    /// Clean up old requests outside the window
    fn cleanup(&mut self, window: Duration) {
        let cutoff = Instant::now() - window;
        self.requests.retain(|&t| t > cutoff);
    }

    /// Check if in cooldown period
    fn check_cooldown(&mut self, cooldown: Duration) -> bool {
        if let Some(last) = self.last_limited {
            if last.elapsed() < cooldown {
                return true;
            } else {
                self.in_cooldown = false;
            }
        }
        false
    }
}

/// Rate limit check result
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    /// Whether the request is allowed
    pub allowed: bool,
    /// Remaining requests in window
    pub remaining: u32,
    /// Seconds until window resets
    pub reset_after_secs: u64,
    /// Whether user is in cooldown
    pub in_cooldown: bool,
    /// Reason if not allowed
    pub reason: Option<String>,
}

impl RateLimitResult {
    fn allowed(remaining: u32, reset_after: u64) -> Self {
        Self {
            allowed: true,
            remaining,
            reset_after_secs: reset_after,
            in_cooldown: false,
            reason: None,
        }
    }

    fn denied(reason: &str, reset_after: u64, in_cooldown: bool) -> Self {
        Self {
            allowed: false,
            remaining: 0,
            reset_after_secs: reset_after,
            in_cooldown,
            reason: Some(reason.to_string()),
        }
    }
}

/// Channel rate limiter
pub struct ChannelRateLimiter {
    config: RateLimitConfig,
    /// Per-user limits: user_id -> entry
    user_limits: Arc<RwLock<HashMap<String, RateLimitEntry>>>,
    /// Global request counter
    global_requests: Arc<RwLock<Vec<Instant>>>,
    /// Channel name for logging
    channel_name: String,
}

impl ChannelRateLimiter {
    /// Create new rate limiter for a channel
    pub fn new(channel_name: &str, config: RateLimitConfig) -> Self {
        Self {
            config,
            user_limits: Arc::new(RwLock::new(HashMap::new())),
            global_requests: Arc::new(RwLock::new(Vec::new())),
            channel_name: channel_name.to_string(),
        }
    }

    /// Create with default config
    pub fn default_for(channel_name: &str) -> Self {
        Self::new(channel_name, RateLimitConfig::default())
    }

    /// Check if a request from user is allowed
    pub async fn check(&self, user_id: &str) -> RateLimitResult {
        let window = Duration::from_secs(self.config.window_secs);
        let cooldown = Duration::from_secs(self.config.cooldown_secs);
        let max_with_burst = self.config.max_requests + self.config.burst_allowance;

        // Check global limit first
        if self.config.enable_global_limit {
            let mut global = self.global_requests.write().await;
            let cutoff = Instant::now() - window;
            global.retain(|&t| t > cutoff);

            if global.len() >= self.config.global_max_requests as usize {
                warn!(
                    "Channel {} hit global rate limit ({} requests)",
                    self.channel_name,
                    global.len()
                );
                return RateLimitResult::denied(
                    "Channel global rate limit exceeded",
                    self.config.window_secs,
                    false,
                );
            }
        }

        // Check per-user limit
        let mut limits = self.user_limits.write().await;
        let entry = limits
            .entry(user_id.to_string())
            .or_insert_with(RateLimitEntry::new);

        // Check cooldown
        if entry.check_cooldown(cooldown) {
            let remaining_cooldown = cooldown
                .saturating_sub(entry.last_limited.unwrap().elapsed())
                .as_secs();
            return RateLimitResult::denied(
                "Rate limit cooldown active",
                remaining_cooldown,
                true,
            );
        }

        // Cleanup old requests
        entry.cleanup(window);

        // Check if over limit
        if entry.requests.len() >= max_with_burst as usize {
            entry.last_limited = Some(Instant::now());
            entry.in_cooldown = true;

            warn!(
                "User {} rate limited on channel {} ({} requests)",
                user_id,
                self.channel_name,
                entry.requests.len()
            );

            return RateLimitResult::denied(
                "Rate limit exceeded",
                self.config.cooldown_secs,
                true,
            );
        }

        // Allow request
        entry.requests.push(Instant::now());

        // Update global counter
        if self.config.enable_global_limit {
            let mut global = self.global_requests.write().await;
            global.push(Instant::now());
        }

        let remaining = max_with_burst.saturating_sub(entry.requests.len() as u32);
        let oldest = entry.requests.first().copied();
        let reset_after = oldest
            .map(|t| window.saturating_sub(t.elapsed()).as_secs())
            .unwrap_or(self.config.window_secs);

        debug!(
            "User {} allowed on channel {} ({} remaining)",
            user_id, self.channel_name, remaining
        );

        RateLimitResult::allowed(remaining, reset_after)
    }

    /// Record a request (use after check passes)
    pub async fn record(&self, user_id: &str) {
        // Already recorded in check(), this is for explicit recording
        let mut limits = self.user_limits.write().await;
        if let Some(entry) = limits.get_mut(user_id) {
            entry.requests.push(Instant::now());
        }
    }

    /// Reset limits for a user (admin action)
    pub async fn reset_user(&self, user_id: &str) {
        let mut limits = self.user_limits.write().await;
        limits.remove(user_id);
    }

    /// Cleanup expired entries (call periodically)
    pub async fn cleanup(&self) {
        let window = Duration::from_secs(self.config.window_secs);
        let cutoff = Instant::now() - window - Duration::from_secs(60);

        // Cleanup user entries
        let mut limits = self.user_limits.write().await;
        limits.retain(|_, entry| {
            entry.cleanup(window);
            !entry.requests.is_empty() || entry.last_limited.map(|t| t > cutoff).unwrap_or(false)
        });

        // Cleanup global
        if self.config.enable_global_limit {
            let mut global = self.global_requests.write().await;
            let cutoff = Instant::now() - window;
            global.retain(|&t| t > cutoff);
        }
    }

    /// Get current stats
    pub async fn stats(&self) -> RateLimitStats {
        let limits = self.user_limits.read().await;
        let global = self.global_requests.read().await;

        let active_users = limits.len();
        let limited_users = limits.values().filter(|e| e.in_cooldown).count();
        let total_requests: usize = limits.values().map(|e| e.requests.len()).sum();

        RateLimitStats {
            channel: self.channel_name.clone(),
            active_users,
            limited_users,
            total_requests_in_window: total_requests,
            global_requests_in_window: global.len(),
            config: self.config.clone(),
        }
    }
}

/// Rate limit statistics
#[derive(Debug, Clone)]
pub struct RateLimitStats {
    pub channel: String,
    pub active_users: usize,
    pub limited_users: usize,
    pub total_requests_in_window: usize,
    pub global_requests_in_window: usize,
    pub config: RateLimitConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limit_allows_under_limit() {
        let limiter = ChannelRateLimiter::new("test", RateLimitConfig {
            max_requests: 5,
            window_secs: 60,
            burst_allowance: 0,
            cooldown_secs: 30,
            enable_global_limit: false,
            global_max_requests: 1000,
        });

        for i in 0..5 {
            let result = limiter.check("user1").await;
            assert!(result.allowed, "Request {} should be allowed", i);
        }
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_over_limit() {
        let limiter = ChannelRateLimiter::new("test", RateLimitConfig {
            max_requests: 3,
            window_secs: 60,
            burst_allowance: 0,
            cooldown_secs: 30,
            enable_global_limit: false,
            global_max_requests: 1000,
        });

        // Use up limit
        for _ in 0..3 {
            let result = limiter.check("user1").await;
            assert!(result.allowed);
        }

        // Should be blocked
        let result = limiter.check("user1").await;
        assert!(!result.allowed);
        assert!(result.in_cooldown);
    }

    #[tokio::test]
    async fn test_rate_limit_user_isolation() {
        let limiter = ChannelRateLimiter::new("test", RateLimitConfig {
            max_requests: 2,
            window_secs: 60,
            burst_allowance: 0,
            cooldown_secs: 30,
            enable_global_limit: false,
            global_max_requests: 1000,
        });

        // User 1 hits limit
        limiter.check("user1").await;
        limiter.check("user1").await;
        let result = limiter.check("user1").await;
        assert!(!result.allowed);

        // User 2 should still have their limit
        let result = limiter.check("user2").await;
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn test_burst_allowance() {
        let limiter = ChannelRateLimiter::new("test", RateLimitConfig {
            max_requests: 3,
            window_secs: 60,
            burst_allowance: 2,
            cooldown_secs: 30,
            enable_global_limit: false,
            global_max_requests: 1000,
        });

        // Should allow 3 + 2 = 5 requests
        for i in 0..5 {
            let result = limiter.check("user1").await;
            assert!(result.allowed, "Request {} should be allowed with burst", i);
        }

        // 6th should be blocked
        let result = limiter.check("user1").await;
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn test_stats() {
        let limiter = ChannelRateLimiter::new("test", RateLimitConfig::default());

        limiter.check("user1").await;
        limiter.check("user2").await;
        limiter.check("user1").await;

        let stats = limiter.stats().await;
        assert_eq!(stats.active_users, 2);
        assert_eq!(stats.total_requests_in_window, 3);
    }
}
