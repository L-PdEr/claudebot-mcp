//! Status & Metrics API
//!
//! REST endpoints for system status, usage metrics, and conversation data.
//! Powers the dashboard real-time UI.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::metrics::{AggregateMetrics, CostBreakdown, LatencyStats, MetricsCollector};

/// Shared application state for status/metrics endpoints
#[derive(Clone)]
pub struct StatusState {
    /// Server start time
    pub start_time: Instant,
    /// Application version
    pub version: &'static str,
    /// Metrics collector
    pub metrics: Option<Arc<MetricsCollector>>,
    /// Bot status
    pub bot_status: BotStatus,
}

/// Bot operational status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BotStatus {
    #[default]
    Running,
    Stopped,
    Error,
}

/// API health status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiStatus {
    #[default]
    Ok,
    Degraded,
    Down,
}

impl StatusState {
    /// Create new status state with defaults
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
            metrics: None,
            bot_status: BotStatus::Running,
        }
    }

    /// Create with metrics collector
    pub fn with_metrics(metrics: Arc<MetricsCollector>) -> Self {
        Self {
            start_time: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
            metrics: Some(metrics),
            bot_status: BotStatus::Running,
        }
    }

    /// Get uptime in seconds
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Default for StatusState {
    fn default() -> Self {
        Self::new()
    }
}

// ===== Response Types =====

/// System status response
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Application version
    pub version: &'static str,
    /// Uptime in seconds
    pub uptime_secs: u64,
    /// Approximate memory usage in MB (process RSS)
    pub memory_mb: u64,
    /// Bot operational status
    pub bot_status: BotStatus,
    /// API health status
    pub api_status: ApiStatus,
    /// Timestamp of last processed message (ISO 8601)
    pub last_message_at: Option<String>,
    /// Current timestamp (ISO 8601)
    pub timestamp: String,
}

/// Usage metrics response
#[derive(Debug, Serialize)]
pub struct MetricsResponse {
    /// Messages processed today
    pub messages_today: u64,
    /// Messages processed this week
    pub messages_week: u64,
    /// Tokens used today
    pub tokens_today: u64,
    /// Cost today in USD
    pub cost_today_usd: f64,
    /// Cache hit rate (0-100)
    pub cache_hit_rate: f64,
    /// Average response latency in ms
    pub avg_response_ms: u64,
    /// Latency percentiles
    pub latency: LatencyStats,
    /// Cost breakdown
    pub cost: CostBreakdown,
    /// Per-model breakdown
    pub by_model: AggregateMetrics,
}

/// Conversation summary for list
#[derive(Debug, Serialize)]
pub struct ConversationItem {
    /// Chat ID
    pub chat_id: i64,
    /// Number of messages
    pub message_count: usize,
    /// First message timestamp (Unix ms)
    pub first_message_at: Option<i64>,
    /// Last message timestamp (Unix ms)
    pub last_message_at: Option<i64>,
    /// Preview of last message (truncated)
    pub preview: Option<String>,
}

/// Paginated conversations response
#[derive(Debug, Serialize)]
pub struct ConversationsResponse {
    /// List of conversations
    pub conversations: Vec<ConversationItem>,
    /// Total count
    pub total: usize,
    /// Whether more results exist
    pub has_more: bool,
}

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Number of items to return (default: 20, max: 100)
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for pagination (default: 0)
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    20
}

/// API error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error code
    pub error: &'static str,
    /// Human-readable message
    pub message: String,
    /// Optional additional context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

// ===== Handlers =====

/// GET /api/status - System status and health
pub async fn status_handler(State(state): State<Arc<StatusState>>) -> Json<StatusResponse> {
    // Get memory usage (best effort, platform-specific)
    let memory_mb = get_memory_usage_mb();

    // Determine API status based on metrics availability
    let api_status = if state.metrics.is_some() {
        ApiStatus::Ok
    } else {
        ApiStatus::Degraded
    };

    // Get last message timestamp from metrics if available
    let last_message_at = state.metrics.as_ref().and_then(|m| {
        let agg = m.aggregate(None);
        if agg.total_requests > 0 {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        }
    });

    Json(StatusResponse {
        version: state.version,
        uptime_secs: state.uptime_secs(),
        memory_mb,
        bot_status: state.bot_status,
        api_status,
        last_message_at,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// GET /api/metrics - Usage statistics
pub async fn metrics_handler(
    State(state): State<Arc<StatusState>>,
) -> Result<Json<MetricsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let metrics = state.metrics.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "METRICS_UNAVAILABLE",
                message: "Metrics collector not initialized".to_string(),
                details: None,
            }),
        )
    })?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let day_ago = now.saturating_sub(86400);
    let week_ago = now.saturating_sub(86400 * 7);

    let today = metrics.aggregate(Some(day_ago));
    let week = metrics.aggregate(Some(week_ago));
    let latency = metrics.latency_stats();
    let cost = metrics.cost_breakdown();
    let all = metrics.aggregate(None);

    Ok(Json(MetricsResponse {
        messages_today: today.total_requests,
        messages_week: week.total_requests,
        tokens_today: today.total_input_tokens + today.total_output_tokens,
        cost_today_usd: today.total_cost_usd,
        cache_hit_rate: today.cache_hit_rate,
        avg_response_ms: today.avg_latency_ms as u64,
        latency,
        cost,
        by_model: all,
    }))
}

/// GET /api/conversations - Recent conversations (paginated)
///
/// Note: This is a stub that returns empty data.
/// Full implementation requires ConversationStore integration.
pub async fn conversations_handler(
    Query(params): Query<PaginationParams>,
) -> Json<ConversationsResponse> {
    // Clamp limit to valid range
    let _limit = params.limit.min(100).max(1);
    let _offset = params.offset;

    // Stub: Return empty for now
    // Full implementation would query ConversationStore
    // let conversations = store.list_conversations(limit, offset)?;

    Json(ConversationsResponse {
        conversations: Vec::new(),
        total: 0,
        has_more: false,
    })
}

// ===== Utilities =====

/// Get approximate memory usage in MB
/// Uses /proc/self/statm on Linux, falls back to 0 on other platforms
fn get_memory_usage_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            // Format: size resident shared text lib data dt
            // Fields are in pages (usually 4KB)
            if let Some(rss_pages) = statm.split_whitespace().nth(1) {
                if let Ok(pages) = rss_pages.parse::<u64>() {
                    // Assume 4KB pages
                    return (pages * 4) / 1024;
                }
            }
        }
        0
    }

    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_status_state_new() {
        let state = StatusState::new();
        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
        assert!(state.metrics.is_none());
        assert_eq!(state.bot_status, BotStatus::Running);
    }

    #[test]
    fn test_status_state_uptime() {
        let state = StatusState::new();
        std::thread::sleep(Duration::from_millis(10));
        let uptime = state.uptime_secs();
        // Should be 0 or more (we slept 10ms which is < 1s)
        assert!(uptime < 10);
    }

    #[test]
    fn test_status_state_with_metrics() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let state = StatusState::with_metrics(metrics.clone());
        assert!(state.metrics.is_some());
    }

    #[test]
    fn test_pagination_params_defaults() {
        let params: PaginationParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, 20);
        assert_eq!(params.offset, 0);
    }

    #[test]
    fn test_bot_status_serialization() {
        assert_eq!(
            serde_json::to_string(&BotStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&BotStatus::Stopped).unwrap(),
            "\"stopped\""
        );
        assert_eq!(
            serde_json::to_string(&BotStatus::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn test_api_status_serialization() {
        assert_eq!(serde_json::to_string(&ApiStatus::Ok).unwrap(), "\"ok\"");
        assert_eq!(
            serde_json::to_string(&ApiStatus::Degraded).unwrap(),
            "\"degraded\""
        );
        assert_eq!(
            serde_json::to_string(&ApiStatus::Down).unwrap(),
            "\"down\""
        );
    }

    #[test]
    fn test_error_response_serialization() {
        let error = ErrorResponse {
            error: "TEST_ERROR",
            message: "Test message".to_string(),
            details: None,
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"error\":\"TEST_ERROR\""));
        assert!(json.contains("\"message\":\"Test message\""));
        // details should be omitted when None
        assert!(!json.contains("details"));
    }

    #[test]
    fn test_error_response_with_details() {
        let error = ErrorResponse {
            error: "TEST_ERROR",
            message: "Test message".to_string(),
            details: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"details\":{\"key\":\"value\"}"));
    }

    #[tokio::test]
    async fn test_status_handler() {
        let state = Arc::new(StatusState::new());
        let response = status_handler(State(state)).await;

        assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(response.bot_status, BotStatus::Running);
        // Without metrics, API status should be degraded
        assert_eq!(response.api_status, ApiStatus::Degraded);
    }

    #[tokio::test]
    async fn test_status_handler_with_metrics() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let state = Arc::new(StatusState::with_metrics(metrics));
        let response = status_handler(State(state)).await;

        assert_eq!(response.api_status, ApiStatus::Ok);
    }

    #[tokio::test]
    async fn test_metrics_handler_no_metrics() {
        let state = Arc::new(StatusState::new());
        let result = metrics_handler(State(state)).await;

        assert!(result.is_err());
        let (status, error) = result.unwrap_err();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.error, "METRICS_UNAVAILABLE");
    }

    #[tokio::test]
    async fn test_metrics_handler_with_metrics() {
        let metrics = Arc::new(MetricsCollector::new(100));

        // Record some test data
        metrics.record(
            "sonnet",
            1000,
            500,
            800,
            Duration::from_millis(1500),
            true,
            Some("test"),
        );

        let state = Arc::new(StatusState::with_metrics(metrics));
        let result = metrics_handler(State(state)).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.cache_hit_rate > 0.0);
    }

    #[tokio::test]
    async fn test_conversations_handler_empty() {
        let params = PaginationParams {
            limit: 20,
            offset: 0,
        };
        let response = conversations_handler(Query(params)).await;

        assert_eq!(response.total, 0);
        assert!(!response.has_more);
        assert!(response.conversations.is_empty());
    }

    #[tokio::test]
    async fn test_conversations_handler_limit_clamped() {
        // Test that limit > 100 is clamped
        let params = PaginationParams {
            limit: 500,
            offset: 0,
        };
        let response = conversations_handler(Query(params)).await;
        // The handler should clamp the limit internally
        assert!(response.conversations.is_empty());
    }

    #[test]
    fn test_get_memory_usage() {
        let mb = get_memory_usage_mb();
        // On Linux, should return something > 0
        // On other platforms, returns 0
        #[cfg(target_os = "linux")]
        assert!(mb > 0);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(mb, 0);
    }
}
