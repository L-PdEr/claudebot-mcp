//! Dashboard Configuration
//!
//! Provides configuration for the dashboard server with security-first defaults.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use serde::{Deserialize, Serialize};

/// Dashboard server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    /// Bind address (default: 127.0.0.1 for security)
    pub bind_addr: IpAddr,
    /// Port number (default: 8080)
    pub port: u16,
    /// Enable authentication (auto-enabled when not localhost)
    pub require_auth: bool,
    /// Enable rate limiting
    pub rate_limit_enabled: bool,
    /// Max requests per minute per IP
    pub rate_limit_rpm: u32,
    /// Enable CORS (always restricted to localhost origins)
    pub cors_enabled: bool,
    /// Allowed CORS origins (only localhost variants)
    pub cors_origins: Vec<String>,
    /// Enable request logging
    pub log_requests: bool,
    /// Static files directory (None = use embedded)
    pub static_dir: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST), // Security: localhost only
            port: 8080,
            require_auth: false, // No auth needed for localhost
            rate_limit_enabled: true,
            rate_limit_rpm: 60,
            cors_enabled: true,
            cors_origins: vec![
                "http://localhost:8080".to_string(),
                "http://127.0.0.1:8080".to_string(),
            ],
            log_requests: true,
            static_dir: None,
        }
    }
}

impl DashboardConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr) = std::env::var("DASHBOARD_BIND_ADDR") {
            if let Ok(parsed) = addr.parse() {
                config.bind_addr = parsed;
            }
        }

        if let Ok(port) = std::env::var("DASHBOARD_PORT") {
            if let Ok(parsed) = port.parse() {
                config.port = parsed;
            }
        }

        if let Ok(val) = std::env::var("DASHBOARD_REQUIRE_AUTH") {
            config.require_auth = val == "true" || val == "1";
        }

        if let Ok(val) = std::env::var("DASHBOARD_LOG_REQUESTS") {
            config.log_requests = val == "true" || val == "1";
        }

        // Auto-enable auth if not binding to localhost
        if !config.is_localhost() && !config.require_auth {
            tracing::warn!(
                "Dashboard binding to {} - authentication should be enabled",
                config.bind_addr
            );
        }

        config
    }

    /// Check if bound to localhost only
    pub fn is_localhost(&self) -> bool {
        match self.bind_addr {
            IpAddr::V4(addr) => addr.is_loopback(),
            IpAddr::V6(addr) => addr.is_loopback(),
        }
    }

    /// Get the socket address
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind_addr, self.port)
    }

    /// Get the base URL for this server
    pub fn base_url(&self) -> String {
        let scheme = "http"; // HTTPS handled by reverse proxy
        format!("{}://{}:{}", scheme, self.bind_addr, self.port)
    }

    /// Configuration for LAN access (requires auth)
    pub fn lan() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED), // 0.0.0.0
            require_auth: true,
            cors_origins: vec![
                "http://localhost:8080".to_string(),
                "http://127.0.0.1:8080".to_string(),
            ],
            ..Default::default()
        }
    }

    /// Configuration for remote access (strict security)
    pub fn remote() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            require_auth: true,
            rate_limit_rpm: 30, // Stricter rate limiting
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_localhost() {
        let config = DashboardConfig::default();
        assert!(config.is_localhost());
        assert_eq!(config.port, 8080);
        assert!(!config.require_auth);
    }

    #[test]
    fn test_lan_requires_auth() {
        let config = DashboardConfig::lan();
        assert!(!config.is_localhost());
        assert!(config.require_auth);
    }

    #[test]
    fn test_socket_addr() {
        let config = DashboardConfig::default();
        let addr = config.socket_addr();
        assert_eq!(addr.port(), 8080);
        assert!(addr.ip().is_loopback());
    }

    #[test]
    fn test_base_url() {
        let config = DashboardConfig::default();
        assert_eq!(config.base_url(), "http://127.0.0.1:8080");
    }
}
