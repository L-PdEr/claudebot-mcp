//! Prometheus Metrics
//!
//! Exposes metrics for monitoring the bridge system.
//! Metrics are exposed via HTTP endpoint /metrics.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Metric types
#[derive(Debug, Clone, Copy)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
}

/// A single metric value with labels
#[derive(Debug)]
struct MetricValue {
    value: AtomicU64,
    labels: HashMap<String, String>,
}

/// Counter metric (always increases)
#[derive(Debug)]
pub struct Counter {
    name: String,
    help: String,
    values: RwLock<Vec<(HashMap<String, String>, AtomicU64)>>,
}

impl Counter {
    pub fn new(name: &str, help: &str) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            values: RwLock::new(Vec::new()),
        }
    }

    pub async fn inc(&self) {
        self.inc_by(1).await;
    }

    pub async fn inc_by(&self, n: u64) {
        let values = self.values.read().await;
        if let Some((_, counter)) = values.first() {
            counter.fetch_add(n, Ordering::Relaxed);
        } else {
            drop(values);
            let mut values = self.values.write().await;
            if values.is_empty() {
                values.push((HashMap::new(), AtomicU64::new(n)));
            }
        }
    }

    pub async fn inc_with_labels(&self, labels: HashMap<String, String>) {
        let values = self.values.read().await;
        for (l, counter) in values.iter() {
            if l == &labels {
                counter.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        drop(values);

        let mut values = self.values.write().await;
        values.push((labels, AtomicU64::new(1)));
    }

    pub async fn get(&self) -> u64 {
        let values = self.values.read().await;
        values.first().map(|(_, c)| c.load(Ordering::Relaxed)).unwrap_or(0)
    }

    pub async fn format(&self) -> String {
        let values = self.values.read().await;
        let mut output = format!("# HELP {} {}\n", self.name, self.help);
        output.push_str(&format!("# TYPE {} counter\n", self.name));

        if values.is_empty() {
            output.push_str(&format!("{} 0\n", self.name));
        } else {
            for (labels, value) in values.iter() {
                if labels.is_empty() {
                    output.push_str(&format!("{} {}\n", self.name, value.load(Ordering::Relaxed)));
                } else {
                    let label_str: Vec<String> = labels.iter()
                        .map(|(k, v)| format!("{}=\"{}\"", k, v))
                        .collect();
                    output.push_str(&format!("{}{{{}}} {}\n",
                        self.name,
                        label_str.join(","),
                        value.load(Ordering::Relaxed)
                    ));
                }
            }
        }

        output
    }
}

/// Gauge metric (can go up or down)
#[derive(Debug)]
pub struct Gauge {
    name: String,
    help: String,
    value: AtomicU64,
}

impl Gauge {
    pub fn new(name: &str, help: &str) -> Self {
        Self {
            name: name.to_string(),
            help: help.to_string(),
            value: AtomicU64::new(0),
        }
    }

    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn format(&self) -> String {
        format!(
            "# HELP {} {}\n# TYPE {} gauge\n{} {}\n",
            self.name, self.help, self.name, self.name, self.get()
        )
    }
}

/// Histogram for measuring distributions
#[derive(Debug)]
pub struct Histogram {
    name: String,
    help: String,
    buckets: Vec<f64>,
    counts: Vec<AtomicU64>,
    sum: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    pub fn new(name: &str, help: &str, buckets: Vec<f64>) -> Self {
        let counts = buckets.iter().map(|_| AtomicU64::new(0)).collect();
        Self {
            name: name.to_string(),
            help: help.to_string(),
            buckets,
            counts,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, value: f64) {
        // Update buckets
        for (i, bucket) in self.buckets.iter().enumerate() {
            if value <= *bucket {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
            }
        }

        // Update sum and count (store as integer microseconds for atomicity)
        self.sum.fetch_add((value * 1_000_000.0) as u64, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn format(&self) -> String {
        let mut output = format!("# HELP {} {}\n", self.name, self.help);
        output.push_str(&format!("# TYPE {} histogram\n", self.name));

        let mut cumulative = 0u64;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += self.counts[i].load(Ordering::Relaxed);
            output.push_str(&format!("{}_bucket{{le=\"{}\"}} {}\n", self.name, bucket, cumulative));
        }

        output.push_str(&format!("{}_bucket{{le=\"+Inf\"}} {}\n", self.name, self.count.load(Ordering::Relaxed)));
        output.push_str(&format!("{}_sum {}\n", self.name, self.sum.load(Ordering::Relaxed) as f64 / 1_000_000.0));
        output.push_str(&format!("{}_count {}\n", self.name, self.count.load(Ordering::Relaxed)));

        output
    }
}

/// Bridge metrics collector
pub struct BridgeMetrics {
    pub workers_spawned: Counter,
    pub workers_active: Gauge,
    pub tasks_total: Counter,
    pub task_duration: Histogram,
    pub errors_total: Counter,
    pub requests_total: Counter,
    pub circuit_breaker_state: Gauge,
    start_time: Instant,
}

impl BridgeMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            workers_spawned: Counter::new("bridge_workers_spawned_total", "Total workers spawned"),
            workers_active: Gauge::new("bridge_workers_active", "Currently active workers"),
            tasks_total: Counter::new("bridge_tasks_total", "Total tasks processed"),
            task_duration: Histogram::new(
                "bridge_task_duration_seconds",
                "Task execution duration in seconds",
                vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0],
            ),
            errors_total: Counter::new("bridge_errors_total", "Total errors by category"),
            requests_total: Counter::new("bridge_requests_total", "Total gRPC requests"),
            circuit_breaker_state: Gauge::new("bridge_circuit_breaker_state", "Circuit breaker state (0=closed, 1=open, 2=half-open)"),
            start_time: Instant::now(),
        })
    }

    /// Record a task completion
    pub async fn record_task(&self, duration_secs: f64, success: bool) {
        self.tasks_total.inc().await;
        self.task_duration.observe(duration_secs);
        if !success {
            self.errors_total.inc_with_labels(
                [("category".to_string(), "task_failure".to_string())].into_iter().collect()
            ).await;
        }
    }

    /// Record a worker spawn
    pub async fn record_worker_spawn(&self) {
        self.workers_spawned.inc().await;
        self.workers_active.inc();
    }

    /// Record a worker stop
    pub fn record_worker_stop(&self) {
        self.workers_active.dec();
    }

    /// Update circuit breaker state
    pub fn update_circuit_state(&self, state: u64) {
        self.circuit_breaker_state.set(state);
    }

    /// Format all metrics for Prometheus
    pub async fn format_metrics(&self) -> String {
        let mut output = String::new();

        // Uptime
        output.push_str(&format!(
            "# HELP bridge_uptime_seconds Uptime in seconds\n\
             # TYPE bridge_uptime_seconds gauge\n\
             bridge_uptime_seconds {}\n\n",
            self.start_time.elapsed().as_secs()
        ));

        output.push_str(&self.workers_spawned.format().await);
        output.push('\n');
        output.push_str(&self.workers_active.format());
        output.push('\n');
        output.push_str(&self.tasks_total.format().await);
        output.push('\n');
        output.push_str(&self.task_duration.format());
        output.push('\n');
        output.push_str(&self.errors_total.format().await);
        output.push('\n');
        output.push_str(&self.requests_total.format().await);
        output.push('\n');
        output.push_str(&self.circuit_breaker_state.format());

        output
    }
}

impl Default for BridgeMetrics {
    fn default() -> Self {
        Arc::try_unwrap(Self::new()).unwrap_or_else(|arc| (*arc).clone())
    }
}

impl Clone for BridgeMetrics {
    fn clone(&self) -> Self {
        // Metrics should be shared, not cloned
        // This is a simple implementation for demonstration
        Self {
            workers_spawned: Counter::new("bridge_workers_spawned_total", "Total workers spawned"),
            workers_active: Gauge::new("bridge_workers_active", "Currently active workers"),
            tasks_total: Counter::new("bridge_tasks_total", "Total tasks processed"),
            task_duration: Histogram::new(
                "bridge_task_duration_seconds",
                "Task execution duration in seconds",
                vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0],
            ),
            errors_total: Counter::new("bridge_errors_total", "Total errors by category"),
            requests_total: Counter::new("bridge_requests_total", "Total gRPC requests"),
            circuit_breaker_state: Gauge::new("bridge_circuit_breaker_state", "Circuit breaker state"),
            start_time: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_counter() {
        let counter = Counter::new("test_counter", "A test counter");
        counter.inc().await;
        counter.inc().await;
        assert_eq!(counter.get().await, 2);
    }

    #[test]
    fn test_gauge() {
        let gauge = Gauge::new("test_gauge", "A test gauge");
        gauge.set(10);
        assert_eq!(gauge.get(), 10);
        gauge.inc();
        assert_eq!(gauge.get(), 11);
        gauge.dec();
        assert_eq!(gauge.get(), 10);
    }

    #[test]
    fn test_histogram() {
        let histogram = Histogram::new("test_hist", "A test histogram", vec![1.0, 5.0, 10.0]);
        histogram.observe(0.5);
        histogram.observe(3.0);
        histogram.observe(7.0);
        histogram.observe(15.0);

        let formatted = histogram.format();
        assert!(formatted.contains("test_hist_bucket{le=\"1\"} 1"));
        assert!(formatted.contains("test_hist_count 4"));
    }

    #[tokio::test]
    async fn test_metrics_format() {
        let metrics = BridgeMetrics::new();
        metrics.workers_active.set(3);
        metrics.record_task(1.5, true).await;

        let output = metrics.format_metrics().await;
        assert!(output.contains("bridge_workers_active 3"));
        assert!(output.contains("bridge_tasks_total"));
    }
}
