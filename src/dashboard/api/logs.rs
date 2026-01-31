//! Log Viewer API
//!
//! Enhanced log viewing with filtering, search, history retrieval, and download.
//! Builds on top of the SSE streaming in stream.rs.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use futures_util::{stream, StreamExt as FuturesStreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

// ===== Types =====

/// Log component/source
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum LogComponent {
    /// Telegram bot operations
    Telegram,
    /// Claude API interactions
    Claude,
    /// Memory/conversation storage
    Memory,
    /// Skill execution
    Skills,
    /// Dashboard/web server
    Dashboard,
    /// System/general operations
    System,
}

impl LogComponent {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "telegram" => Some(Self::Telegram),
            "claude" => Some(Self::Claude),
            "memory" => Some(Self::Memory),
            "skills" => Some(Self::Skills),
            "dashboard" => Some(Self::Dashboard),
            "system" => Some(Self::System),
            _ => None,
        }
    }

    /// All component variants
    pub fn all() -> &'static [LogComponent] {
        &[
            Self::Telegram,
            Self::Claude,
            Self::Memory,
            Self::Skills,
            Self::Dashboard,
            Self::System,
        ]
    }
}

impl std::fmt::Display for LogComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Telegram => write!(f, "telegram"),
            Self::Claude => write!(f, "claude"),
            Self::Memory => write!(f, "memory"),
            Self::Skills => write!(f, "skills"),
            Self::Dashboard => write!(f, "dashboard"),
            Self::System => write!(f, "system"),
        }
    }
}

/// Log level (mirrors stream.rs LogLevel but with ordering for filtering)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

impl LogLevel {
    /// Parse from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// All level variants
    pub fn all() -> &'static [LogLevel] {
        &[Self::Debug, Self::Info, Self::Warn, Self::Error]
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

/// Enhanced log entry with component information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unique entry ID
    pub id: u64,
    /// Log level
    pub level: LogLevel,
    /// Component that generated the log
    pub component: LogComponent,
    /// Log message
    pub message: String,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
    /// Optional structured metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(
        id: u64,
        level: LogLevel,
        component: LogComponent,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id,
            level,
            component,
            message: message.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            metadata: None,
        }
    }

    /// Create with metadata
    pub fn with_metadata(
        id: u64,
        level: LogLevel,
        component: LogComponent,
        message: impl Into<String>,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            id,
            level,
            component,
            message: message.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            metadata: Some(metadata),
        }
    }

    /// Check if entry matches filter criteria
    pub fn matches_filter(&self, filter: &LogFilter) -> bool {
        // Level filter (show this level and higher severity)
        if let Some(min_level) = filter.min_level {
            if self.level < min_level {
                return false;
            }
        }

        // Component filter
        if let Some(ref components) = filter.components {
            if !components.contains(&self.component) {
                return false;
            }
        }

        // Search filter (case-insensitive)
        if let Some(ref search) = filter.search {
            let search_lower = search.to_lowercase();
            if !self.message.to_lowercase().contains(&search_lower) {
                return false;
            }
        }

        true
    }

    /// Format as plain text log line
    pub fn to_log_line(&self) -> String {
        format!(
            "{} {:5} {:9} {}",
            &self.timestamp[11..19], // HH:MM:SS
            self.level,
            self.component,
            self.message
        )
    }
}

/// Log filter criteria
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilter {
    /// Minimum log level to include
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_level: Option<LogLevel>,
    /// Components to include (None = all)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<LogComponent>>,
    /// Text search within message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

// ===== State =====

/// Log viewer state with history buffer
pub struct LogApiState {
    /// Ring buffer of recent logs
    history: RwLock<VecDeque<LogEntry>>,
    /// Maximum history size
    max_history: usize,
    /// Next entry ID counter
    next_id: RwLock<u64>,
    /// Broadcast channel for new logs
    log_tx: broadcast::Sender<LogEntry>,
}

impl LogApiState {
    /// Create new log state with specified history size
    pub fn new(max_history: usize) -> Self {
        let (log_tx, _) = broadcast::channel(1000);
        Self {
            history: RwLock::new(VecDeque::with_capacity(max_history)),
            max_history,
            next_id: RwLock::new(1),
            log_tx,
        }
    }

    /// Create with default 10,000 line history
    pub fn with_defaults() -> Self {
        Self::new(10_000)
    }

    /// Add a log entry
    pub fn log(
        &self,
        level: LogLevel,
        component: LogComponent,
        message: impl Into<String>,
    ) -> LogEntry {
        let id = {
            let mut next = self.next_id.write();
            let id = *next;
            *next += 1;
            id
        };

        let entry = LogEntry::new(id, level, component, message);

        // Add to history
        {
            let mut history = self.history.write();
            if history.len() >= self.max_history {
                history.pop_front();
            }
            history.push_back(entry.clone());
        }

        // Broadcast to subscribers (ignore errors if no subscribers)
        let _ = self.log_tx.send(entry.clone());

        entry
    }

    /// Add a log entry with metadata
    pub fn log_with_metadata(
        &self,
        level: LogLevel,
        component: LogComponent,
        message: impl Into<String>,
        metadata: serde_json::Value,
    ) -> LogEntry {
        let id = {
            let mut next = self.next_id.write();
            let id = *next;
            *next += 1;
            id
        };

        let entry = LogEntry::with_metadata(id, level, component, message, metadata);

        // Add to history
        {
            let mut history = self.history.write();
            if history.len() >= self.max_history {
                history.pop_front();
            }
            history.push_back(entry.clone());
        }

        // Broadcast to subscribers
        let _ = self.log_tx.send(entry.clone());

        entry
    }

    /// Get log history, optionally filtered
    pub fn get_history(&self, filter: &LogFilter, limit: usize, offset: usize) -> Vec<LogEntry> {
        let history = self.history.read();

        history
            .iter()
            .rev() // Most recent first
            .filter(|e: &&LogEntry| e.matches_filter(filter))
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get total count matching filter
    pub fn count(&self, filter: &LogFilter) -> usize {
        let history = self.history.read();
        history.iter().filter(|e: &&LogEntry| e.matches_filter(filter)).count()
    }

    /// Subscribe to new log entries
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.log_tx.subscribe()
    }

    /// Clear all logs
    pub fn clear(&self) {
        self.history.write().clear();
    }

    /// Export all logs as plain text
    pub fn export_text(&self, filter: &LogFilter) -> String {
        let history = self.history.read();
        history
            .iter()
            .filter(|e: &&LogEntry| e.matches_filter(filter))
            .map(|e: &LogEntry| e.to_log_line())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Export all logs as JSON
    pub fn export_json(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let history = self.history.read();
        history
            .iter()
            .filter(|e: &&LogEntry| e.matches_filter(filter))
            .cloned()
            .collect()
    }

    /// Get stats about current log buffer
    pub fn stats(&self) -> LogStats {
        let history = self.history.read();

        let mut by_level = std::collections::HashMap::new();
        let mut by_component = std::collections::HashMap::new();

        for entry in history.iter() {
            *by_level.entry(entry.level).or_insert(0u64) += 1;
            *by_component.entry(entry.component).or_insert(0u64) += 1;
        }

        LogStats {
            total: history.len() as u64,
            max_capacity: self.max_history as u64,
            by_level,
            by_component,
            oldest_timestamp: history.front().map(|e| e.timestamp.clone()),
            newest_timestamp: history.back().map(|e| e.timestamp.clone()),
        }
    }

    // Convenience methods for common log operations
    pub fn debug(&self, component: LogComponent, message: impl Into<String>) -> LogEntry {
        self.log(LogLevel::Debug, component, message)
    }

    pub fn info(&self, component: LogComponent, message: impl Into<String>) -> LogEntry {
        self.log(LogLevel::Info, component, message)
    }

    pub fn warn(&self, component: LogComponent, message: impl Into<String>) -> LogEntry {
        self.log(LogLevel::Warn, component, message)
    }

    pub fn error(&self, component: LogComponent, message: impl Into<String>) -> LogEntry {
        self.log(LogLevel::Error, component, message)
    }
}

impl Default for LogApiState {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ===== API Types =====

/// Log history query parameters
#[derive(Debug, Deserialize)]
pub struct LogHistoryQuery {
    /// Minimum log level
    #[serde(default)]
    pub level: Option<String>,
    /// Components to include (comma-separated)
    #[serde(default)]
    pub components: Option<String>,
    /// Text search
    #[serde(default)]
    pub search: Option<String>,
    /// Maximum entries to return (default: 100, max: 1000)
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for pagination
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    100
}

impl LogHistoryQuery {
    fn to_filter(&self) -> LogFilter {
        LogFilter {
            min_level: self.level.as_ref().and_then(|l| LogLevel::from_str(l)),
            components: self.components.as_ref().map(|c| {
                c.split(',')
                    .filter_map(|s| LogComponent::from_str(s.trim()))
                    .collect()
            }),
            search: self.search.clone(),
        }
    }
}

/// Log stream query parameters
#[derive(Debug, Deserialize)]
pub struct LogStreamQuery {
    /// Minimum log level
    #[serde(default)]
    pub level: Option<String>,
    /// Components to include (comma-separated)
    #[serde(default)]
    pub components: Option<String>,
    /// Text search
    #[serde(default)]
    pub search: Option<String>,
}

impl LogStreamQuery {
    fn to_filter(&self) -> LogFilter {
        LogFilter {
            min_level: self.level.as_ref().and_then(|l| LogLevel::from_str(l)),
            components: self.components.as_ref().map(|c| {
                c.split(',')
                    .filter_map(|s| LogComponent::from_str(s.trim()))
                    .collect()
            }),
            search: self.search.clone(),
        }
    }
}

/// Log download query parameters
#[derive(Debug, Deserialize)]
pub struct LogDownloadQuery {
    /// Minimum log level
    #[serde(default)]
    pub level: Option<String>,
    /// Components to include (comma-separated)
    #[serde(default)]
    pub components: Option<String>,
    /// Search filter
    #[serde(default)]
    pub search: Option<String>,
    /// Format: "text" or "json" (default: text)
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "text".to_string()
}

impl LogDownloadQuery {
    fn to_filter(&self) -> LogFilter {
        LogFilter {
            min_level: self.level.as_ref().and_then(|l| LogLevel::from_str(l)),
            components: self.components.as_ref().map(|c| {
                c.split(',')
                    .filter_map(|s| LogComponent::from_str(s.trim()))
                    .collect()
            }),
            search: self.search.clone(),
        }
    }
}

/// Log history response
#[derive(Debug, Serialize)]
pub struct LogHistoryResponse {
    /// Log entries (most recent first)
    pub entries: Vec<LogEntry>,
    /// Total matching entries
    pub total: usize,
    /// Whether there are more entries
    pub has_more: bool,
    /// Applied filter
    pub filter: LogFilter,
}

/// Log statistics
#[derive(Debug, Serialize)]
pub struct LogStats {
    /// Total entries in buffer
    pub total: u64,
    /// Maximum buffer capacity
    pub max_capacity: u64,
    /// Count by level
    pub by_level: std::collections::HashMap<LogLevel, u64>,
    /// Count by component
    pub by_component: std::collections::HashMap<LogComponent, u64>,
    /// Oldest entry timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_timestamp: Option<String>,
    /// Newest entry timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest_timestamp: Option<String>,
}

// ===== Handlers =====

/// GET /api/logs - Get log history with filtering
pub async fn get_logs(
    State(state): State<Arc<LogApiState>>,
    Query(query): Query<LogHistoryQuery>,
) -> Json<LogHistoryResponse> {
    let filter = query.to_filter();
    let limit = query.limit.min(1000);

    let total = state.count(&filter);
    let entries = state.get_history(&filter, limit, query.offset);
    let has_more = query.offset + entries.len() < total;

    Json(LogHistoryResponse {
        entries,
        total,
        has_more,
        filter,
    })
}

/// GET /api/logs/stats - Get log buffer statistics
pub async fn get_stats(State(state): State<Arc<LogApiState>>) -> Json<LogStats> {
    Json(state.stats())
}

/// GET /api/logs/stream - SSE stream of new logs with filtering
pub async fn stream_logs(
    State(state): State<Arc<LogApiState>>,
    Query(query): Query<LogStreamQuery>,
) -> Response {
    let filter = query.to_filter();
    let rx = state.subscribe();
    let broadcast_stream = BroadcastStream::new(rx);

    // Process broadcast stream: convert to SSE events with filtering
    // Use map to transform each result, then use filter to remove None entries
    let log_stream = broadcast_stream
        .map(move |result| {
            let filter = &filter;
            match result {
                Ok(ref entry) if entry.matches_filter(filter) => Some(Ok::<_, Infallible>(Event::default()
                    .event("log")
                    .data(serde_json::to_string(&entry).unwrap_or_default())
                    .id(entry.id.to_string()))),
                Ok(_) => None, // Filtered out
                Err(_) => Some(Ok(Event::default()
                    .event("warning")
                    .data(r#"{"message":"Some logs were skipped due to buffer overflow"}"#))),
            }
        })
        .filter(|opt: &Option<Result<Event, Infallible>>| futures_util::future::ready(opt.is_some()))
        .map(|opt: Option<Result<Event, Infallible>>| opt.unwrap());

    // Add heartbeat stream
    let heartbeat = stream::unfold((), |()| async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let event = Event::default()
            .event("heartbeat")
            .data(format!(r#"{{"timestamp":"{}"}}"#, chrono::Utc::now().to_rfc3339()));
        Some((Ok::<_, Infallible>(event), ()))
    });

    let combined = stream::select(log_stream, heartbeat);

    Sse::new(combined)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(30))
                .text("heartbeat"),
        )
        .into_response()
}

/// GET /api/logs/download - Download logs as file
pub async fn download_logs(
    State(state): State<Arc<LogApiState>>,
    Query(query): Query<LogDownloadQuery>,
) -> Response {
    let filter = query.to_filter();
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");

    match query.format.as_str() {
        "json" => {
            let entries = state.export_json(&filter);
            let content = serde_json::to_string_pretty(&entries).unwrap_or_default();

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"claudebot_logs_{}.json\"", timestamp),
                )
                .body(content.into())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        _ => {
            // Default to text
            let content = state.export_text(&filter);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"claudebot_logs_{}.txt\"", timestamp),
                )
                .body(content.into())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// DELETE /api/logs - Clear log buffer
pub async fn clear_logs(State(state): State<Arc<LogApiState>>) -> StatusCode {
    state.clear();
    StatusCode::NO_CONTENT
}

// ===== Router =====

/// Create the logs API router
pub fn logs_router(state: Arc<LogApiState>) -> Router {
    Router::new()
        .route("/", get(get_logs).delete(clear_logs))
        .route("/stats", get(get_stats))
        .route("/stream", get(stream_logs))
        .route("/download", get(download_logs))
        .with_state(state)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::body::Body;
    use tower::ServiceExt;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("debug"), Some(LogLevel::Debug));
        assert_eq!(LogLevel::from_str("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::from_str("Warning"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_str("ERROR"), Some(LogLevel::Error));
        assert_eq!(LogLevel::from_str("invalid"), None);
    }

    #[test]
    fn test_log_component_from_str() {
        assert_eq!(LogComponent::from_str("telegram"), Some(LogComponent::Telegram));
        assert_eq!(LogComponent::from_str("CLAUDE"), Some(LogComponent::Claude));
        assert_eq!(LogComponent::from_str("invalid"), None);
    }

    #[test]
    fn test_log_entry_matches_filter() {
        let entry = LogEntry::new(1, LogLevel::Info, LogComponent::Telegram, "Test message");

        // No filter matches all
        assert!(entry.matches_filter(&LogFilter::default()));

        // Level filter
        let filter = LogFilter {
            min_level: Some(LogLevel::Debug),
            ..Default::default()
        };
        assert!(entry.matches_filter(&filter));

        let filter = LogFilter {
            min_level: Some(LogLevel::Warn),
            ..Default::default()
        };
        assert!(!entry.matches_filter(&filter));

        // Component filter
        let filter = LogFilter {
            components: Some(vec![LogComponent::Telegram]),
            ..Default::default()
        };
        assert!(entry.matches_filter(&filter));

        let filter = LogFilter {
            components: Some(vec![LogComponent::Claude]),
            ..Default::default()
        };
        assert!(!entry.matches_filter(&filter));

        // Search filter
        let filter = LogFilter {
            search: Some("test".to_string()),
            ..Default::default()
        };
        assert!(entry.matches_filter(&filter));

        let filter = LogFilter {
            search: Some("notfound".to_string()),
            ..Default::default()
        };
        assert!(!entry.matches_filter(&filter));
    }

    #[test]
    fn test_log_state_basic() {
        let state = LogApiState::new(100);

        let entry = state.info(LogComponent::System, "Test log");
        assert_eq!(entry.id, 1);
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.component, LogComponent::System);

        let entry2 = state.error(LogComponent::Claude, "Error log");
        assert_eq!(entry2.id, 2);
    }

    #[test]
    fn test_log_state_history() {
        let state = LogApiState::new(100);

        state.info(LogComponent::Telegram, "Message 1");
        state.warn(LogComponent::Claude, "Message 2");
        state.error(LogComponent::System, "Message 3");

        let history = state.get_history(&LogFilter::default(), 10, 0);
        assert_eq!(history.len(), 3);
        // Most recent first
        assert_eq!(history[0].message, "Message 3");
        assert_eq!(history[2].message, "Message 1");
    }

    #[test]
    fn test_log_state_history_limit() {
        let state = LogApiState::new(5);

        for i in 1..=10 {
            state.info(LogComponent::System, format!("Message {}", i));
        }

        let stats = state.stats();
        assert_eq!(stats.total, 5);

        let history = state.get_history(&LogFilter::default(), 10, 0);
        // Should only have last 5 messages (6-10)
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].message, "Message 10");
        assert_eq!(history[4].message, "Message 6");
    }

    #[test]
    fn test_log_state_filtered_history() {
        let state = LogApiState::new(100);

        state.debug(LogComponent::Telegram, "Debug telegram");
        state.info(LogComponent::Claude, "Info claude");
        state.warn(LogComponent::Memory, "Warn memory");
        state.error(LogComponent::Skills, "Error skills");

        // Filter by level
        let filter = LogFilter {
            min_level: Some(LogLevel::Warn),
            ..Default::default()
        };
        let history = state.get_history(&filter, 10, 0);
        assert_eq!(history.len(), 2);

        // Filter by component
        let filter = LogFilter {
            components: Some(vec![LogComponent::Telegram, LogComponent::Claude]),
            ..Default::default()
        };
        let history = state.get_history(&filter, 10, 0);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_log_export_text() {
        let state = LogApiState::new(100);

        state.info(LogComponent::Telegram, "Test message");

        let text = state.export_text(&LogFilter::default());
        assert!(text.contains("INFO"));
        assert!(text.contains("telegram"));
        assert!(text.contains("Test message"));
    }

    #[test]
    fn test_log_stats() {
        let state = LogApiState::new(100);

        state.info(LogComponent::Telegram, "Msg 1");
        state.info(LogComponent::Telegram, "Msg 2");
        state.warn(LogComponent::Claude, "Msg 3");
        state.error(LogComponent::Claude, "Msg 4");

        let stats = state.stats();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.by_level.get(&LogLevel::Info), Some(&2));
        assert_eq!(stats.by_level.get(&LogLevel::Warn), Some(&1));
        assert_eq!(stats.by_level.get(&LogLevel::Error), Some(&1));
        assert_eq!(stats.by_component.get(&LogComponent::Telegram), Some(&2));
        assert_eq!(stats.by_component.get(&LogComponent::Claude), Some(&2));
    }

    #[tokio::test]
    async fn test_get_logs_endpoint() {
        let state = Arc::new(LogApiState::new(100));
        state.info(LogComponent::System, "Test log");

        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total"], 1);
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn test_get_logs_filtered() {
        let state = Arc::new(LogApiState::new(100));
        state.debug(LogComponent::Telegram, "Debug msg");
        state.info(LogComponent::Claude, "Info msg");
        state.warn(LogComponent::Memory, "Warn msg");

        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/?level=warn&components=memory")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total"], 1);
    }

    #[tokio::test]
    async fn test_get_stats_endpoint() {
        let state = Arc::new(LogApiState::new(100));
        state.info(LogComponent::System, "Test");

        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total"], 1);
        assert_eq!(json["max_capacity"], 100);
    }

    #[tokio::test]
    async fn test_download_logs_text() {
        let state = Arc::new(LogApiState::new(100));
        state.info(LogComponent::System, "Test log");

        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/download?format=text")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/plain"));
        assert!(response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("attachment"));
    }

    #[tokio::test]
    async fn test_download_logs_json() {
        let state = Arc::new(LogApiState::new(100));
        state.info(LogComponent::System, "Test log");

        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/download?format=json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("application/json"));
    }

    #[tokio::test]
    async fn test_clear_logs() {
        let state = Arc::new(LogApiState::new(100));
        state.info(LogComponent::System, "Test log 1");
        state.info(LogComponent::System, "Test log 2");

        assert_eq!(state.stats().total, 2);

        let app = logs_router(state.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(state.stats().total, 0);
    }

    #[tokio::test]
    async fn test_stream_logs_endpoint() {
        let state = Arc::new(LogApiState::new(100));
        let app = logs_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/event-stream"));
    }
}
