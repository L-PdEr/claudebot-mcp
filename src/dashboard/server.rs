//! Dashboard HTTP Server
//!
//! Axum-based server with embedded static files, CORS, and graceful shutdown.

use crate::dashboard::api::{health_router, health::AppState};
use crate::dashboard::config::DashboardConfig;
use axum::{
    body::Body,
    http::{header, Method, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::Embed;
use std::sync::Arc;
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// Embedded static files for the dashboard
#[derive(Embed)]
#[folder = "src/dashboard/static/"]
struct StaticAssets;

/// Dashboard server
pub struct DashboardServer {
    config: DashboardConfig,
    state: Arc<AppState>,
}

impl DashboardServer {
    /// Create a new dashboard server with the given configuration
    pub fn new(config: DashboardConfig) -> Self {
        Self {
            config,
            state: Arc::new(AppState::new()),
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(DashboardConfig::default())
    }

    /// Build the router with all routes and middleware
    fn build_router(&self) -> Router {
        // CORS configuration - localhost only for security
        let cors = if self.config.cors_enabled {
            CorsLayer::new()
                .allow_origin(
                    self.config
                        .cors_origins
                        .iter()
                        .filter_map(|o| o.parse().ok())
                        .collect::<Vec<_>>(),
                )
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        } else {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET])
                .allow_headers([header::CONTENT_TYPE])
        };

        // Build router
        let mut router = Router::new()
            // Static file serving
            .route("/", get(index_handler))
            .route("/{*path}", get(static_handler))
            // Health API (nested)
            .nest("/api", health_router(self.state.clone()))
            // Middleware
            .layer(cors);

        // Add request logging if enabled
        if self.config.log_requests {
            router = router.layer(TraceLayer::new_for_http());
        }

        router
    }

    /// Start the server and run until shutdown signal
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = self.config.socket_addr();
        let router = self.build_router();

        info!("Starting dashboard server on {}", addr);

        if self.config.is_localhost() {
            info!("Dashboard bound to localhost - no authentication required");
        } else {
            warn!(
                "Dashboard bound to {} - ensure authentication is configured",
                addr
            );
        }

        info!("Dashboard available at {}", self.config.base_url());

        // Create the server with graceful shutdown
        let listener = tokio::net::TcpListener::bind(addr).await?;

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        info!("Dashboard server shut down gracefully");
        Ok(())
    }

    /// Get the configuration
    pub fn config(&self) -> &DashboardConfig {
        &self.config
    }
}

/// Serve the index.html file
async fn index_handler() -> impl IntoResponse {
    match StaticAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => {
            // Fallback minimal HTML if no index.html embedded
            Html(FALLBACK_INDEX).into_response()
        }
    }
}

/// Serve static files from embedded assets
async fn static_handler(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');

    // Security: prevent path traversal
    if path.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Graceful shutdown signal handler
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, initiating graceful shutdown");
        }
        _ = terminate => {
            info!("Received SIGTERM, initiating graceful shutdown");
        }
    }
}

/// Fallback index page when no static files are embedded
const FALLBACK_INDEX: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>ClaudeBot Dashboard</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #0f172a;
            color: #e2e8f0;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
        }
        .container {
            text-align: center;
            padding: 2rem;
        }
        h1 { font-size: 2.5rem; margin-bottom: 1rem; color: #38bdf8; }
        p { color: #94a3b8; margin-bottom: 2rem; }
        .status {
            display: inline-flex;
            align-items: center;
            gap: 0.5rem;
            padding: 0.5rem 1rem;
            background: #1e293b;
            border-radius: 9999px;
            font-size: 0.875rem;
        }
        .status-dot {
            width: 8px;
            height: 8px;
            background: #22c55e;
            border-radius: 50%;
            animation: pulse 2s infinite;
        }
        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.5; }
        }
        .links { margin-top: 2rem; }
        .links a {
            color: #38bdf8;
            text-decoration: none;
            margin: 0 1rem;
        }
        .links a:hover { text-decoration: underline; }
    </style>
</head>
<body>
    <div class="container">
        <h1>ClaudeBot Dashboard</h1>
        <p>The dashboard UI is not yet installed.</p>
        <div class="status">
            <span class="status-dot"></span>
            Server Running
        </div>
        <div class="links">
            <a href="/api/health">Health Check</a>
            <a href="/api/healthz">Liveness</a>
            <a href="/api/readyz">Readiness</a>
        </div>
    </div>
    <script>
        // Auto-refresh status
        fetch('/api/health')
            .then(r => r.json())
            .then(data => {
                console.log('Server health:', data);
            })
            .catch(err => {
                document.querySelector('.status-dot').style.background = '#ef4444';
            });
    </script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
        assert!(json["uptime_secs"].is_number());
    }

    #[tokio::test]
    async fn test_liveness_endpoint() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_readiness_endpoint() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_index_returns_html() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("ClaudeBot Dashboard"));
    }

    #[tokio::test]
    async fn test_path_traversal_blocked() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/../../etc/passwd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_unknown_file_returns_404() {
        let server = DashboardServer::with_defaults();
        let app = server.build_router();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nonexistent.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
