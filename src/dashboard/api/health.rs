//! Health Check API
//!
//! Provides health check endpoint for monitoring and load balancers.

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    /// Server start time for uptime calculation
    pub start_time: Instant,
    /// Application version
    pub version: &'static str,
}

impl AppState {
    /// Create new application state
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Get uptime in seconds
    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Health status
    pub status: &'static str,
    /// Application version
    pub version: &'static str,
    /// Uptime in seconds
    pub uptime_secs: u64,
    /// Timestamp (ISO 8601)
    pub timestamp: String,
}

/// Health check handler
///
/// Returns 200 OK with health information.
/// Used by load balancers and monitoring systems.
pub async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: state.version,
        uptime_secs: state.uptime_secs(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

/// Liveness probe (minimal response)
///
/// Returns 200 OK if the server is alive.
/// Kubernetes-style liveness check.
pub async fn liveness() -> StatusCode {
    StatusCode::OK
}

/// Readiness probe
///
/// Returns 200 OK if the server is ready to accept traffic.
/// Can be extended to check database connections, etc.
pub async fn readiness() -> StatusCode {
    // Future: Check database connections, external services
    StatusCode::OK
}

/// Create health check router
pub fn health_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_uptime() {
        let state = AppState::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        // uptime_secs returns u64, always >= 0
        let uptime = state.uptime_secs();
        assert!(uptime < 10); // Should be less than 10 seconds
    }

    #[test]
    fn test_app_state_version() {
        let state = AppState::new();
        assert!(!state.version.is_empty());
    }
}
