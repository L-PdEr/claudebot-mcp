//! Dashboard API Endpoints
//!
//! REST API for dashboard functionality.

pub mod health;
pub mod status;
pub mod stream;

use axum::{routing::get, Router};
use std::sync::Arc;

pub use health::health_router;
pub use status::{
    conversations_handler, metrics_handler, status_handler, ApiStatus, BotStatus,
    ConversationItem, ConversationsResponse, ErrorResponse, MetricsResponse, PaginationParams,
    StatusResponse, StatusState,
};
pub use stream::{
    stream_logs, stream_messages, stream_metrics, stream_router, HeartbeatEvent, LogEvent,
    LogLevel, MessageEvent, MetricsEvent, StreamState,
};

/// Combined dashboard API state
#[derive(Clone)]
pub struct DashboardApiState {
    /// Health check state
    pub health: Arc<health::AppState>,
    /// Status/metrics state
    pub status: Arc<StatusState>,
}

impl DashboardApiState {
    /// Create new dashboard API state with defaults
    pub fn new() -> Self {
        Self {
            health: Arc::new(health::AppState::new()),
            status: Arc::new(StatusState::new()),
        }
    }

    /// Create with metrics collector
    pub fn with_metrics(metrics: Arc<crate::metrics::MetricsCollector>) -> Self {
        Self {
            health: Arc::new(health::AppState::new()),
            status: Arc::new(StatusState::with_metrics(metrics)),
        }
    }
}

impl Default for DashboardApiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the full API router with all endpoints
///
/// Routes:
/// - GET /health - Health check with version and uptime
/// - GET /healthz - Kubernetes liveness probe
/// - GET /readyz - Kubernetes readiness probe
/// - GET /status - System status and bot state
/// - GET /metrics - Usage statistics and cost breakdown
/// - GET /conversations - Paginated conversation list
pub fn api_router(state: DashboardApiState) -> Router {
    // Health endpoints router
    let health_router = Router::new()
        .route("/health", get(health::health_check))
        .route("/healthz", get(health::liveness))
        .route("/readyz", get(health::readiness))
        .with_state(state.health);

    // Status/metrics endpoints router
    let status_router = Router::new()
        .route("/status", get(status_handler))
        .route("/metrics", get(metrics_handler))
        .route("/conversations", get(conversations_handler))
        .with_state(state.status);

    // Merge the routers
    health_router.merge(status_router)
}

/// Create a minimal API router with just health endpoints
pub fn minimal_api_router(state: Arc<health::AppState>) -> Router {
    health_router(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    #[test]
    fn test_dashboard_api_state_new() {
        let state = DashboardApiState::new();
        assert!(state.status.metrics.is_none());
    }

    #[test]
    fn test_dashboard_api_state_with_metrics() {
        let metrics = Arc::new(crate::metrics::MetricsCollector::new(100));
        let state = DashboardApiState::with_metrics(metrics.clone());
        assert!(state.status.metrics.is_some());
    }

    #[tokio::test]
    async fn test_api_router_health() {
        let state = DashboardApiState::new();
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_router_status() {
        let state = DashboardApiState::new();
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["version"].is_string());
        assert!(json["uptime_secs"].is_number());
        assert_eq!(json["bot_status"], "running");
    }

    #[tokio::test]
    async fn test_api_router_metrics_no_collector() {
        let state = DashboardApiState::new();
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Without metrics collector, should return 503
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn test_api_router_metrics_with_collector() {
        let metrics = Arc::new(crate::metrics::MetricsCollector::new(100));
        let state = DashboardApiState::with_metrics(metrics);
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["messages_today"].is_number());
        assert!(json["cost_today_usd"].is_number());
    }

    #[tokio::test]
    async fn test_api_router_conversations() {
        let state = DashboardApiState::new();
        let app = api_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/conversations?limit=10&offset=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["conversations"].is_array());
        assert!(json["total"].is_number());
        assert!(json["has_more"].is_boolean());
    }
}
