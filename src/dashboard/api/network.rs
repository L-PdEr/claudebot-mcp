//! Network Status API
//!
//! Provides information about network configuration including Tailscale status.

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::process::Command;
use std::sync::Arc;

// ===== Types =====

/// Tailscale status response
#[derive(Debug, Serialize)]
pub struct TailscaleStatus {
    /// Whether Tailscale is installed
    pub installed: bool,
    /// Whether Tailscale daemon is running
    pub running: bool,
    /// Whether authenticated to a Tailnet
    pub authenticated: bool,
    /// Hostname on the Tailnet
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Tailnet name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailnet: Option<String>,
    /// Tailscale IPv4 address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// Full URL for dashboard access
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether Tailscale serve is configured
    pub serve_enabled: bool,
    /// Error message if status check failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for TailscaleStatus {
    fn default() -> Self {
        Self {
            installed: false,
            running: false,
            authenticated: false,
            hostname: None,
            tailnet: None,
            ip: None,
            url: None,
            serve_enabled: false,
            error: None,
        }
    }
}

/// Network status response
#[derive(Debug, Serialize)]
pub struct NetworkStatus {
    /// Dashboard binding address
    pub bind_address: String,
    /// Dashboard port
    pub port: u16,
    /// Whether bound to localhost only
    pub localhost_only: bool,
    /// Tailscale status
    pub tailscale: TailscaleStatus,
}

/// Network API state
pub struct NetworkApiState {
    /// Dashboard bind address
    pub bind_address: String,
    /// Dashboard port
    pub port: u16,
}

impl NetworkApiState {
    /// Create new network API state
    pub fn new(bind_address: impl Into<String>, port: u16) -> Self {
        Self {
            bind_address: bind_address.into(),
            port,
        }
    }

    /// Create with default localhost binding
    pub fn with_defaults() -> Self {
        Self::new("127.0.0.1", 8080)
    }
}

impl Default for NetworkApiState {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ===== Tailscale Detection =====

/// Check if Tailscale is installed
fn is_tailscale_installed() -> bool {
    Command::new("tailscale")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if Tailscale daemon is running
fn is_tailscaled_running() -> bool {
    Command::new("tailscale")
        .arg("status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get Tailscale status JSON
fn get_tailscale_status_json() -> Option<serde_json::Value> {
    Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
}

/// Get Tailscale IPv4 address
fn get_tailscale_ip() -> Option<String> {
    Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Check if Tailscale serve is enabled
fn is_serve_enabled() -> bool {
    Command::new("tailscale")
        .args(["serve", "status"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Get full Tailscale status
fn get_tailscale_status() -> TailscaleStatus {
    let installed = is_tailscale_installed();
    if !installed {
        return TailscaleStatus {
            installed: false,
            ..Default::default()
        };
    }

    let running = is_tailscaled_running();
    if !running {
        return TailscaleStatus {
            installed: true,
            running: false,
            ..Default::default()
        };
    }

    // Get detailed status
    let status_json = get_tailscale_status_json();
    let ip = get_tailscale_ip();
    let serve_enabled = is_serve_enabled();

    let (hostname, tailnet, authenticated) = if let Some(ref json) = status_json {
        let dns_name = json
            .get("Self")
            .and_then(|s| s.get("DNSName"))
            .and_then(|d| d.as_str())
            .map(|s| s.trim_end_matches('.').to_string());

        // Extract hostname and tailnet from DNS name
        let (hostname, tailnet) = if let Some(ref dns) = dns_name {
            let parts: Vec<&str> = dns.splitn(2, '.').collect();
            if parts.len() == 2 {
                (Some(parts[0].to_string()), Some(parts[1].to_string()))
            } else {
                (Some(dns.clone()), None)
            }
        } else {
            (None, None)
        };

        // Check if authenticated (has a valid DNS name)
        let authenticated = dns_name.is_some();

        (hostname, tailnet, authenticated)
    } else {
        (None, None, false)
    };

    // Build URL if we have hostname
    let url = hostname.as_ref().zip(tailnet.as_ref()).map(|(h, t)| {
        format!("https://{}.{}", h, t)
    });

    TailscaleStatus {
        installed: true,
        running: true,
        authenticated,
        hostname,
        tailnet,
        ip,
        url,
        serve_enabled,
        error: None,
    }
}

// ===== Handlers =====

/// GET /api/network - Get network status
pub async fn get_network_status(
    State(state): State<Arc<NetworkApiState>>,
) -> Json<NetworkStatus> {
    let localhost_only = state.bind_address == "127.0.0.1" || state.bind_address == "localhost";

    Json(NetworkStatus {
        bind_address: state.bind_address.clone(),
        port: state.port,
        localhost_only,
        tailscale: get_tailscale_status(),
    })
}

/// GET /api/network/tailscale - Get Tailscale status only
pub async fn get_tailscale(
    State(_state): State<Arc<NetworkApiState>>,
) -> Json<TailscaleStatus> {
    Json(get_tailscale_status())
}

// ===== Router =====

/// Create the network API router
pub fn network_router(state: Arc<NetworkApiState>) -> Router {
    Router::new()
        .route("/", get(get_network_status))
        .route("/tailscale", get(get_tailscale))
        .with_state(state)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[test]
    fn test_network_api_state_defaults() {
        let state = NetworkApiState::with_defaults();
        assert_eq!(state.bind_address, "127.0.0.1");
        assert_eq!(state.port, 8080);
    }

    #[test]
    fn test_network_api_state_custom() {
        let state = NetworkApiState::new("0.0.0.0", 3000);
        assert_eq!(state.bind_address, "0.0.0.0");
        assert_eq!(state.port, 3000);
    }

    #[test]
    fn test_tailscale_status_default() {
        let status = TailscaleStatus::default();
        assert!(!status.installed);
        assert!(!status.running);
        assert!(!status.authenticated);
        assert!(status.hostname.is_none());
    }

    #[tokio::test]
    async fn test_get_network_status_localhost() {
        let state = Arc::new(NetworkApiState::with_defaults());
        let app = network_router(state);

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

        assert_eq!(json["bind_address"], "127.0.0.1");
        assert_eq!(json["port"], 8080);
        assert_eq!(json["localhost_only"], true);
    }

    #[tokio::test]
    async fn test_get_network_status_all_interfaces() {
        let state = Arc::new(NetworkApiState::new("0.0.0.0", 8080));
        let app = network_router(state);

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

        assert_eq!(json["bind_address"], "0.0.0.0");
        assert_eq!(json["localhost_only"], false);
    }

    #[tokio::test]
    async fn test_get_tailscale_endpoint() {
        let state = Arc::new(NetworkApiState::with_defaults());
        let app = network_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/tailscale")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // These will depend on whether Tailscale is installed on the test system
        assert!(json["installed"].is_boolean());
        assert!(json["running"].is_boolean());
    }

    #[test]
    fn test_tailscale_status_serialization() {
        let status = TailscaleStatus {
            installed: true,
            running: true,
            authenticated: true,
            hostname: Some("claudebot".to_string()),
            tailnet: Some("example.ts.net".to_string()),
            ip: Some("100.64.1.1".to_string()),
            url: Some("https://claudebot.example.ts.net".to_string()),
            serve_enabled: true,
            error: None,
        };

        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["installed"], true);
        assert_eq!(json["hostname"], "claudebot");
        assert_eq!(json["url"], "https://claudebot.example.ts.net");
        // Error should be omitted when None
        assert!(json.get("error").is_none());
    }

    #[test]
    fn test_tailscale_status_with_error() {
        let status = TailscaleStatus {
            installed: true,
            running: false,
            authenticated: false,
            error: Some("Daemon not running".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["error"], "Daemon not running");
    }
}
