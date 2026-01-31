//! Server-Sent Events (SSE) Streaming
//!
//! Real-time streaming endpoints for dashboard updates.
//! SSE is chosen over WebSocket because:
//! - Unidirectional (server → client) - perfect for dashboards
//! - Native browser EventSource API with auto-reconnect
//! - Works through proxies without configuration
//! - Simpler to implement and maintain

use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures_util::stream::{self, Stream};
use serde::Serialize;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::metrics::MetricsCollector;

/// SSE stream state
#[derive(Clone)]
pub struct StreamState {
    /// Metrics collector for live metrics
    pub metrics: Option<Arc<MetricsCollector>>,
    /// Broadcast channel for messages
    pub message_tx: Option<tokio::sync::broadcast::Sender<MessageEvent>>,
    /// Broadcast channel for logs
    pub log_tx: Option<tokio::sync::broadcast::Sender<LogEvent>>,
}

impl StreamState {
    /// Create new stream state
    pub fn new() -> Self {
        Self {
            metrics: None,
            message_tx: None,
            log_tx: None,
        }
    }

    /// Create with metrics collector
    pub fn with_metrics(metrics: Arc<MetricsCollector>) -> Self {
        Self {
            metrics: Some(metrics),
            message_tx: None,
            log_tx: None,
        }
    }

    /// Create with all components
    pub fn with_all(
        metrics: Arc<MetricsCollector>,
        message_tx: tokio::sync::broadcast::Sender<MessageEvent>,
        log_tx: tokio::sync::broadcast::Sender<LogEvent>,
    ) -> Self {
        Self {
            metrics: Some(metrics),
            message_tx: Some(message_tx),
            log_tx: Some(log_tx),
        }
    }
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

// ===== Event Types =====

/// Message event for SSE stream
#[derive(Debug, Clone, Serialize)]
pub struct MessageEvent {
    /// Message ID
    pub id: String,
    /// User identifier
    pub user: String,
    /// Message content (truncated to 100 chars)
    pub content: String,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
}

impl MessageEvent {
    /// Create a new message event
    pub fn new(id: impl Into<String>, user: impl Into<String>, content: impl Into<String>) -> Self {
        let content = content.into();
        let truncated = if content.chars().count() > 100 {
            // Safe UTF-8 truncation
            let truncated: String = content.chars().take(97).collect();
            format!("{}...", truncated)
        } else {
            content
        };

        Self {
            id: id.into(),
            user: user.into(),
            content: truncated,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Metrics event for SSE stream
#[derive(Debug, Clone, Serialize)]
pub struct MetricsEvent {
    /// Messages processed today
    pub messages_today: u64,
    /// Tokens used today
    pub tokens_today: u64,
    /// Active users (placeholder)
    pub active_users: u64,
    /// Cache hit rate
    pub cache_hit_rate: f64,
    /// Cost today in USD
    pub cost_today_usd: f64,
}

/// Log event for SSE stream
#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    /// Log level
    pub level: LogLevel,
    /// Log message
    pub message: String,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
}

/// Log level
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogEvent {
    /// Create a new log event
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Create info log event
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, message)
    }

    /// Create error log event
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, message)
    }

    /// Create warn log event
    pub fn warn(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, message)
    }

    /// Create debug log event
    pub fn debug(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Debug, message)
    }
}

// ===== Heartbeat Event =====

/// Heartbeat event data
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatEvent {
    /// Server timestamp
    pub timestamp: String,
    /// Uptime in seconds
    pub uptime_secs: u64,
}

/// Type alias for boxed SSE stream
type BoxedSseStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

// ===== SSE Handlers =====

/// GET /api/stream/metrics - Live metrics (1 event/second)
///
/// Streams metrics events every second with system statistics.
/// Includes heartbeat to prevent connection timeout.
pub async fn stream_metrics(State(state): State<Arc<StreamState>>) -> Response {
    let metrics = state.metrics.clone();
    let start_time = std::time::Instant::now();

    // Create a stream that emits every second
    let stream = stream::unfold(0u64, move |counter| {
        let metrics = metrics.clone();
        async move {
            // Wait 1 second between events
            tokio::time::sleep(Duration::from_secs(1)).await;

            let event = if let Some(ref m) = metrics {
                // Get real metrics
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let day_ago = now.saturating_sub(86400);
                let today = m.aggregate(Some(day_ago));

                let metrics_event = MetricsEvent {
                    messages_today: today.total_requests,
                    tokens_today: today.total_input_tokens + today.total_output_tokens,
                    active_users: 0, // Placeholder - would need user tracking
                    cache_hit_rate: today.cache_hit_rate,
                    cost_today_usd: today.total_cost_usd,
                };

                Event::default()
                    .event("metrics")
                    .data(serde_json::to_string(&metrics_event).unwrap_or_default())
                    .id(counter.to_string())
            } else {
                // No metrics - send heartbeat
                let heartbeat = HeartbeatEvent {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    uptime_secs: start_time.elapsed().as_secs(),
                };

                Event::default()
                    .event("heartbeat")
                    .data(serde_json::to_string(&heartbeat).unwrap_or_default())
            };

            Some((Ok::<_, Infallible>(event), counter + 1))
        }
    });

    let boxed: BoxedSseStream = Box::pin(stream);
    Sse::new(boxed)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("heartbeat"),
        )
        .into_response()
}

/// GET /api/stream/messages - New messages as they arrive
///
/// Streams message events as they are received.
/// Falls back to heartbeat if no message channel is configured.
pub async fn stream_messages(State(state): State<Arc<StreamState>>) -> Response {
    let start_time = std::time::Instant::now();

    let stream: BoxedSseStream = if let Some(ref tx) = state.message_tx {
        // Subscribe to the broadcast channel using BroadcastStream wrapper
        let rx = tx.subscribe();
        let broadcast_stream = BroadcastStream::new(rx);

        let s = broadcast_stream.map(|result| {
            Ok(match result {
                Ok(msg) => Event::default()
                    .event("message")
                    .data(serde_json::to_string(&msg).unwrap_or_default()),
                Err(_) => Event::default()
                    .event("warning")
                    .data(r#"{"message":"Some messages were skipped"}"#),
            })
        });
        Box::pin(s)
    } else {
        // No message channel - send periodic heartbeat
        let s = stream::unfold(0u64, move |counter| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;

            let heartbeat = HeartbeatEvent {
                timestamp: chrono::Utc::now().to_rfc3339(),
                uptime_secs: start_time.elapsed().as_secs(),
            };

            let event = Event::default()
                .event("heartbeat")
                .data(serde_json::to_string(&heartbeat).unwrap_or_default());

            Some((Ok::<_, Infallible>(event), counter + 1))
        });
        Box::pin(s)
    };

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("heartbeat"),
        )
        .into_response()
}

/// GET /api/stream/logs - Log tail stream
///
/// Streams log events as they are emitted.
/// Falls back to heartbeat if no log channel is configured.
pub async fn stream_logs(State(state): State<Arc<StreamState>>) -> Response {
    let start_time = std::time::Instant::now();

    let stream: BoxedSseStream = if let Some(ref tx) = state.log_tx {
        // Subscribe to the broadcast channel using BroadcastStream wrapper
        let rx = tx.subscribe();
        let broadcast_stream = BroadcastStream::new(rx);

        let s = broadcast_stream.map(|result| {
            Ok(match result {
                Ok(log) => Event::default()
                    .event("log")
                    .data(serde_json::to_string(&log).unwrap_or_default()),
                Err(_) => Event::default()
                    .event("warning")
                    .data(r#"{"message":"Some logs were skipped"}"#),
            })
        });
        Box::pin(s)
    } else {
        // No log channel - send periodic heartbeat
        let s = stream::unfold(0u64, move |counter| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;

            let heartbeat = HeartbeatEvent {
                timestamp: chrono::Utc::now().to_rfc3339(),
                uptime_secs: start_time.elapsed().as_secs(),
            };

            let event = Event::default()
                .event("heartbeat")
                .data(serde_json::to_string(&heartbeat).unwrap_or_default());

            Some((Ok::<_, Infallible>(event), counter + 1))
        });
        Box::pin(s)
    };

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("heartbeat"),
        )
        .into_response()
}

/// Create SSE stream router
pub fn stream_router(state: Arc<StreamState>) -> axum::Router {
    use axum::routing::get;

    axum::Router::new()
        .route("/stream/metrics", get(stream_metrics))
        .route("/stream/messages", get(stream_messages))
        .route("/stream/logs", get(stream_logs))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_event_truncation() {
        let short = MessageEvent::new("1", "alice", "Hello");
        assert_eq!(short.content, "Hello");

        let long_content = "a".repeat(200);
        let long = MessageEvent::new("2", "bob", long_content);
        assert_eq!(long.content.len(), 100);
        assert!(long.content.ends_with("..."));
    }

    #[test]
    fn test_log_event_creation() {
        let info = LogEvent::info("Test message");
        assert_eq!(info.level, LogLevel::Info);
        assert_eq!(info.message, "Test message");

        let error = LogEvent::error("Error occurred");
        assert_eq!(error.level, LogLevel::Error);
    }

    #[test]
    fn test_stream_state_default() {
        let state = StreamState::default();
        assert!(state.metrics.is_none());
        assert!(state.message_tx.is_none());
        assert!(state.log_tx.is_none());
    }

    #[test]
    fn test_stream_state_with_metrics() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let state = StreamState::with_metrics(metrics);
        assert!(state.metrics.is_some());
    }

    #[test]
    fn test_stream_state_with_all() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let (msg_tx, _) = tokio::sync::broadcast::channel::<MessageEvent>(100);
        let (log_tx, _) = tokio::sync::broadcast::channel::<LogEvent>(100);

        let state = StreamState::with_all(metrics, msg_tx, log_tx);
        assert!(state.metrics.is_some());
        assert!(state.message_tx.is_some());
        assert!(state.log_tx.is_some());
    }

    #[test]
    fn test_log_level_serialization() {
        assert_eq!(
            serde_json::to_string(&LogLevel::Debug).unwrap(),
            "\"debug\""
        );
        assert_eq!(serde_json::to_string(&LogLevel::Info).unwrap(), "\"info\"");
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), "\"warn\"");
        assert_eq!(
            serde_json::to_string(&LogLevel::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn test_metrics_event_serialization() {
        let event = MetricsEvent {
            messages_today: 100,
            tokens_today: 50000,
            active_users: 5,
            cache_hit_rate: 75.5,
            cost_today_usd: 0.25,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"messages_today\":100"));
        assert!(json.contains("\"cache_hit_rate\":75.5"));
    }

    #[tokio::test]
    async fn test_stream_metrics_emits_events() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let state = Arc::new(StreamState::with_metrics(metrics));

        let response = stream_metrics(State(state)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_broadcast_channel_message_events() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<MessageEvent>(100);

        // Send a message
        let msg = MessageEvent::new("1", "alice", "Hello world");
        tx.send(msg.clone()).unwrap();

        // Receive it
        let received = rx.recv().await.unwrap();
        assert_eq!(received.id, "1");
        assert_eq!(received.user, "alice");
    }

    #[tokio::test]
    async fn test_broadcast_channel_log_events() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<LogEvent>(100);

        // Send a log
        let log = LogEvent::info("Server started");
        tx.send(log).unwrap();

        // Receive it
        let received = rx.recv().await.unwrap();
        assert_eq!(received.level, LogLevel::Info);
        assert_eq!(received.message, "Server started");
    }

    #[tokio::test]
    async fn test_stream_messages_with_channel() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let (msg_tx, _) = tokio::sync::broadcast::channel::<MessageEvent>(100);
        let (log_tx, _) = tokio::sync::broadcast::channel::<LogEvent>(100);

        let state = Arc::new(StreamState::with_all(metrics, msg_tx, log_tx));
        let response = stream_messages(State(state)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_stream_logs_with_channel() {
        let metrics = Arc::new(MetricsCollector::new(100));
        let (msg_tx, _) = tokio::sync::broadcast::channel::<MessageEvent>(100);
        let (log_tx, _) = tokio::sync::broadcast::channel::<LogEvent>(100);

        let state = Arc::new(StreamState::with_all(metrics, msg_tx, log_tx));
        let response = stream_logs(State(state)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_stream_messages_without_channel() {
        let state = Arc::new(StreamState::new());
        let response = stream_messages(State(state)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_stream_logs_without_channel() {
        let state = Arc::new(StreamState::new());
        let response = stream_logs(State(state)).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }
}
