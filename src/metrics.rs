//! Metrics & Monitoring (E6)
//!
//! Cost tracking, latency metrics, and usage statistics.
//! Provides observability into Claude API usage and system performance.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

/// Pricing per million tokens (USD)
/// Constants defined at compile time for zero runtime cost
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cached_input: f64, // 90% discount on cached
}

impl ModelPricing {
    pub const HAIKU: Self = Self {
        input: 0.25,
        output: 1.25,
        cached_input: 0.025, // 90% off
    };

    pub const SONNET: Self = Self {
        input: 3.0,
        output: 15.0,
        cached_input: 0.3,
    };

    pub const OPUS: Self = Self {
        input: 15.0,
        output: 75.0,
        cached_input: 1.5,
    };

    /// Get pricing for model name
    /// Optimized: checks bytes directly instead of allocating lowercase string
    #[inline]
    pub fn for_model(model: &str) -> Self {
        // Check for model identifiers without allocation
        let bytes = model.as_bytes();
        for window in bytes.windows(5) {
            if window.eq_ignore_ascii_case(b"haiku") {
                return Self::HAIKU;
            }
        }
        for window in bytes.windows(4) {
            if window.eq_ignore_ascii_case(b"opus") {
                return Self::OPUS;
            }
        }
        Self::SONNET
    }

    /// Calculate cost for tokens
    /// Uses multiply-then-divide to minimize floating point operations
    #[inline]
    pub fn calculate_cost(
        &self,
        input_tokens: usize,
        output_tokens: usize,
        cached_tokens: usize,
    ) -> f64 {
        const DIVISOR: f64 = 1_000_000.0;
        let uncached_input = input_tokens.saturating_sub(cached_tokens) as f64;

        // Single division at the end for better numerical stability
        (uncached_input * self.input
            + cached_tokens as f64 * self.cached_input
            + output_tokens as f64 * self.output)
            / DIVISOR
    }
}

/// Single request metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetrics {
    pub timestamp: u64,
    pub model: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cached_tokens: usize,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub cache_hit: bool,
    pub route_target: Option<String>,
}

/// Aggregate metrics for a time period
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregateMetrics {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cached_tokens: u64,
    pub total_cost_usd: f64,
    pub avg_latency_ms: f64,
    pub cache_hit_rate: f64,
    pub by_model: HashMap<String, ModelMetrics>,
}

/// Per-model metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelMetrics {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: f64,
}

/// Latency percentiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
    pub min_ms: u64,
}

/// Cost breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub today_usd: f64,
    pub this_week_usd: f64,
    pub this_month_usd: f64,
    pub by_model: HashMap<String, f64>,
    pub savings_from_cache_usd: f64,
}

/// Real-time metrics collector
pub struct MetricsCollector {
    /// Request history (rolling window)
    requests: Arc<RwLock<Vec<RequestMetrics>>>,
    /// Maximum history size
    max_history: usize,
    /// Counters for fast access
    total_requests: AtomicU64,
    total_cost_micros: AtomicU64, // Store as microdollars for atomic
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

impl MetricsCollector {
    /// Create new metrics collector
    pub fn new(max_history: usize) -> Self {
        Self {
            requests: Arc::new(RwLock::new(Vec::with_capacity(max_history))),
            max_history,
            total_requests: AtomicU64::new(0),
            total_cost_micros: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }

    /// Record a request
    pub fn record(
        &self,
        model: &str,
        input_tokens: usize,
        output_tokens: usize,
        cached_tokens: usize,
        latency: Duration,
        cache_hit: bool,
        route_target: Option<&str>,
    ) {
        let pricing = ModelPricing::for_model(model);
        let cost = pricing.calculate_cost(input_tokens, output_tokens, cached_tokens);

        let metrics = RequestMetrics {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cached_tokens,
            cost_usd: cost,
            latency_ms: latency.as_millis() as u64,
            cache_hit,
            route_target: route_target.map(String::from),
        };

        // Update counters
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_cost_micros
            .fetch_add((cost * 1_000_000.0) as u64, Ordering::Relaxed);

        if cache_hit {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        // Store in history
        if let Ok(mut requests) = self.requests.write() {
            requests.push(metrics);

            // Trim if over capacity
            if requests.len() > self.max_history {
                let drain_count = requests.len() - self.max_history;
                requests.drain(0..drain_count);
            }
        }

        debug!(
            "Recorded request: model={}, tokens={}/{}, cost=${:.6}, latency={}ms",
            model, input_tokens, output_tokens, cost, latency.as_millis()
        );
    }

    /// Get quick stats
    pub fn quick_stats(&self) -> QuickStats {
        let total = self.total_requests.load(Ordering::Relaxed);
        let cost_micros = self.total_cost_micros.load(Ordering::Relaxed);
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);

        QuickStats {
            total_requests: total,
            total_cost_usd: cost_micros as f64 / 1_000_000.0,
            cache_hit_rate: if hits + misses > 0 {
                hits as f64 / (hits + misses) as f64 * 100.0
            } else {
                0.0
            },
        }
    }

    /// Get aggregate metrics for a time window
    pub fn aggregate(&self, since: Option<u64>) -> AggregateMetrics {
        let requests = self.requests.read().ok();
        let requests = match requests {
            Some(r) => r,
            None => return AggregateMetrics::default(),
        };

        let filtered: Vec<_> = if let Some(since_ts) = since {
            requests.iter().filter(|r| r.timestamp >= since_ts).collect()
        } else {
            requests.iter().collect()
        };

        if filtered.is_empty() {
            return AggregateMetrics::default();
        }

        let total_requests = filtered.len() as u64;
        let total_input_tokens: u64 = filtered.iter().map(|r| r.input_tokens as u64).sum();
        let total_output_tokens: u64 = filtered.iter().map(|r| r.output_tokens as u64).sum();
        let total_cached_tokens: u64 = filtered.iter().map(|r| r.cached_tokens as u64).sum();
        let total_cost_usd: f64 = filtered.iter().map(|r| r.cost_usd).sum();
        let total_latency: u64 = filtered.iter().map(|r| r.latency_ms).sum();
        let cache_hits = filtered.iter().filter(|r| r.cache_hit).count();

        // By model breakdown
        let mut by_model: HashMap<String, ModelMetrics> = HashMap::new();
        for r in &filtered {
            let entry = by_model.entry(r.model.clone()).or_default();
            entry.requests += 1;
            entry.input_tokens += r.input_tokens as u64;
            entry.output_tokens += r.output_tokens as u64;
            entry.cached_tokens += r.cached_tokens as u64;
            entry.cost_usd += r.cost_usd;
        }

        AggregateMetrics {
            total_requests,
            total_input_tokens,
            total_output_tokens,
            total_cached_tokens,
            total_cost_usd,
            avg_latency_ms: total_latency as f64 / total_requests as f64,
            cache_hit_rate: cache_hits as f64 / total_requests as f64 * 100.0,
            by_model,
        }
    }

    /// Get latency statistics
    pub fn latency_stats(&self) -> LatencyStats {
        let requests = self.requests.read().ok();
        let requests = match requests {
            Some(r) => r,
            None => {
                return LatencyStats {
                    p50_ms: 0,
                    p90_ms: 0,
                    p99_ms: 0,
                    max_ms: 0,
                    min_ms: 0,
                }
            }
        };

        if requests.is_empty() {
            return LatencyStats {
                p50_ms: 0,
                p90_ms: 0,
                p99_ms: 0,
                max_ms: 0,
                min_ms: 0,
            };
        }

        let mut latencies: Vec<u64> = requests.iter().map(|r| r.latency_ms).collect();
        latencies.sort_unstable();

        let len = latencies.len();
        let p50_idx = len / 2;
        let p90_idx = len * 90 / 100;
        let p99_idx = len * 99 / 100;

        LatencyStats {
            p50_ms: latencies.get(p50_idx).copied().unwrap_or(0),
            p90_ms: latencies.get(p90_idx).copied().unwrap_or(0),
            p99_ms: latencies.get(p99_idx).copied().unwrap_or(0),
            max_ms: latencies.last().copied().unwrap_or(0),
            min_ms: latencies.first().copied().unwrap_or(0),
        }
    }

    /// Get cost breakdown
    pub fn cost_breakdown(&self) -> CostBreakdown {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let day_ago = now.saturating_sub(86400);
        let week_ago = now.saturating_sub(86400 * 7);
        let month_ago = now.saturating_sub(86400 * 30);

        let requests = self.requests.read().ok();
        let requests = match requests {
            Some(r) => r,
            None => {
                return CostBreakdown {
                    today_usd: 0.0,
                    this_week_usd: 0.0,
                    this_month_usd: 0.0,
                    by_model: HashMap::new(),
                    savings_from_cache_usd: 0.0,
                }
            }
        };

        let today_usd: f64 = requests
            .iter()
            .filter(|r| r.timestamp >= day_ago)
            .map(|r| r.cost_usd)
            .sum();

        let this_week_usd: f64 = requests
            .iter()
            .filter(|r| r.timestamp >= week_ago)
            .map(|r| r.cost_usd)
            .sum();

        let this_month_usd: f64 = requests
            .iter()
            .filter(|r| r.timestamp >= month_ago)
            .map(|r| r.cost_usd)
            .sum();

        // By model
        let mut by_model: HashMap<String, f64> = HashMap::new();
        for r in requests.iter() {
            *by_model.entry(r.model.clone()).or_default() += r.cost_usd;
        }

        // Calculate cache savings
        let savings_from_cache_usd: f64 = requests
            .iter()
            .map(|r| {
                let pricing = ModelPricing::for_model(&r.model);
                let would_cost = r.cached_tokens as f64 / 1_000_000.0 * pricing.input;
                let actual_cost = r.cached_tokens as f64 / 1_000_000.0 * pricing.cached_input;
                would_cost - actual_cost
            })
            .sum();

        CostBreakdown {
            today_usd,
            this_week_usd,
            this_month_usd,
            by_model,
            savings_from_cache_usd,
        }
    }

    /// Reset all metrics
    pub fn reset(&self) {
        if let Ok(mut requests) = self.requests.write() {
            requests.clear();
        }
        self.total_requests.store(0, Ordering::Relaxed);
        self.total_cost_micros.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        info!("Metrics reset");
    }

    /// Export metrics as JSON
    pub fn export_json(&self) -> String {
        let stats = self.quick_stats();
        let aggregate = self.aggregate(None);
        let latency = self.latency_stats();
        let cost = self.cost_breakdown();

        serde_json::json!({
            "quick": stats,
            "aggregate": aggregate,
            "latency": latency,
            "cost": cost
        })
        .to_string()
    }
}

/// Quick access stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickStats {
    pub total_requests: u64,
    pub total_cost_usd: f64,
    pub cache_hit_rate: f64,
}

/// Timer for measuring operation latency
pub struct LatencyTimer {
    start: Instant,
    operation: String,
}

impl LatencyTimer {
    pub fn new(operation: &str) -> Self {
        Self {
            start: Instant::now(),
            operation: operation.to_string(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn finish(self) -> Duration {
        let elapsed = self.start.elapsed();
        debug!("{}: {}ms", self.operation, elapsed.as_millis());
        elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_calculation() {
        let sonnet = ModelPricing::SONNET;

        // 1000 input, 500 output, 800 cached
        let cost = sonnet.calculate_cost(1000, 500, 800);

        // Uncached: 200 tokens * $3/M = $0.0006
        // Cached: 800 tokens * $0.30/M = $0.00024
        // Output: 500 tokens * $15/M = $0.0075
        // Total: ~$0.00834
        assert!(cost > 0.008 && cost < 0.009);
    }

    #[test]
    fn test_metrics_recording() {
        let collector = MetricsCollector::new(100);

        collector.record(
            "claude-3-5-sonnet",
            1000,
            500,
            800,
            Duration::from_millis(1500),
            true,
            Some("api"),
        );

        collector.record(
            "claude-3-haiku",
            500,
            200,
            0,
            Duration::from_millis(500),
            false,
            Some("backend"),
        );

        let stats = collector.quick_stats();
        assert_eq!(stats.total_requests, 2);
        assert!(stats.total_cost_usd > 0.0);
        assert_eq!(stats.cache_hit_rate, 50.0);
    }

    #[test]
    fn test_latency_percentiles() {
        let collector = MetricsCollector::new(100);

        // Add some requests with varying latencies
        for ms in [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000] {
            collector.record(
                "sonnet",
                100,
                50,
                0,
                Duration::from_millis(ms),
                false,
                None,
            );
        }

        let stats = collector.latency_stats();
        assert_eq!(stats.min_ms, 100);
        assert_eq!(stats.max_ms, 1000);
        assert!(stats.p50_ms >= 500 && stats.p50_ms <= 600);
    }

    #[test]
    fn test_model_pricing_lookup() {
        assert_eq!(ModelPricing::for_model("claude-3-5-haiku-20241022").input, 0.25);
        assert_eq!(ModelPricing::for_model("claude-3-opus-20240229").input, 15.0);
        assert_eq!(ModelPricing::for_model("claude-sonnet-4-20250514").input, 3.0);
    }

    #[test]
    fn test_aggregate_by_model() {
        let collector = MetricsCollector::new(100);

        collector.record("haiku", 100, 50, 0, Duration::from_millis(100), false, None);
        collector.record("haiku", 100, 50, 0, Duration::from_millis(100), false, None);
        collector.record("sonnet", 200, 100, 0, Duration::from_millis(200), false, None);

        let agg = collector.aggregate(None);
        assert_eq!(agg.total_requests, 3);
        assert_eq!(agg.by_model.get("haiku").unwrap().requests, 2);
        assert_eq!(agg.by_model.get("sonnet").unwrap().requests, 1);
    }

    #[test]
    fn test_empty_collector() {
        let collector = MetricsCollector::new(100);

        let stats = collector.quick_stats();
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.total_cost_usd, 0.0);
        assert_eq!(stats.cache_hit_rate, 0.0);

        let latency = collector.latency_stats();
        assert_eq!(latency.p50_ms, 0);
        assert_eq!(latency.max_ms, 0);
    }

    #[test]
    fn test_reset() {
        let collector = MetricsCollector::new(100);

        collector.record("sonnet", 1000, 500, 0, Duration::from_millis(1000), false, None);
        assert!(collector.quick_stats().total_requests > 0);

        collector.reset();

        assert_eq!(collector.quick_stats().total_requests, 0);
        assert_eq!(collector.quick_stats().total_cost_usd, 0.0);
    }

    #[test]
    fn test_cache_savings_calculation() {
        let collector = MetricsCollector::new(100);

        // Record request with cached tokens
        collector.record(
            "sonnet",
            1000,
            100,
            800, // 800 tokens were cached
            Duration::from_millis(500),
            true,
            None,
        );

        let cost = collector.cost_breakdown();

        // Should have positive savings from cache
        assert!(cost.savings_from_cache_usd > 0.0);
    }

    #[test]
    fn test_rolling_window() {
        let collector = MetricsCollector::new(5); // Small window

        // Add more than capacity
        for i in 0..10 {
            collector.record(
                "haiku",
                100,
                50,
                0,
                Duration::from_millis(i as u64 * 100),
                false,
                None,
            );
        }

        // Should only keep last 5
        let agg = collector.aggregate(None);
        assert!(agg.total_requests <= 5);
    }

    #[test]
    fn test_zero_tokens() {
        let pricing = ModelPricing::SONNET;
        let cost = pricing.calculate_cost(0, 0, 0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_opus_pricing() {
        let pricing = ModelPricing::OPUS;

        // 1M input + 1M output
        let cost = pricing.calculate_cost(1_000_000, 1_000_000, 0);

        // Should be $15 + $75 = $90
        assert!((cost - 90.0).abs() < 0.01);
    }
}
