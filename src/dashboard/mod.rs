//! Web Dashboard Module
//!
//! Provides a secure, local-first web interface for ClaudeBot management.
//!
//! # Security Model
//!
//! - **Localhost binding by default**: No auth needed when bound to 127.0.0.1
//! - **LAN access**: Requires authentication when bound to 0.0.0.0
//! - **Remote access**: Zero-trust via Tailscale + OAuth + MFA
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           Dashboard Server              │
//! ├─────────────────────────────────────────┤
//! │  GET /                 → Static files   │
//! │  GET /api/health       → Health check   │
//! │  GET /api/status       → System status  │
//! │  GET /api/metrics      → Usage metrics  │
//! │  GET /api/stream/*     → SSE streams    │
//! └─────────────────────────────────────────┘
//! ```

pub mod config;
pub mod server;
pub mod api;

pub use config::DashboardConfig;
pub use server::DashboardServer;
pub use api::{
    api_router, health_router, ApiStatus, BotStatus, DashboardApiState, ErrorResponse,
    MetricsResponse, StatusResponse, StatusState,
};
