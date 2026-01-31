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
//! │  GET /api/logs         → Log history    │
//! │  GET /api/logs/stream  → Log tail (SSE) │
//! │  GET /api/logs/download→ Export logs    │
//! │  GET /api/network      → Network status │
//! │  GET /api/network/tailscale → Tailscale │
//! │  POST /api/auth/login  → Authenticate   │
//! │  POST /api/auth/logout → End session    │
//! │  POST /api/auth/refresh→ Refresh token  │
//! │  GET /api/auth/me      → Current user   │
//! └─────────────────────────────────────────┘
//! ```

pub mod api;
pub mod auth;
pub mod config;
pub mod server;

pub use api::{
    api_router, config_router, health_router, logs_router, network_router, skills_router,
    stream_router, users_router, ApiStatus, BotStatus, ConfigApiState, ConfigFieldResponse,
    ConfigFieldSchema, ConfigResponse, ConfigSource, DashboardApiState, EnhancedLogLevel,
    ErrorResponse, FieldSensitivity, FieldType, HeartbeatEvent, InstallSkillRequest,
    InstallSkillResponse, LogApiState, LogComponent, LogEntry, LogEvent, LogFilter,
    LogHistoryResponse, LogLevel, LogStats, MessageEvent, MetricsEvent, MetricsResponse,
    NetworkApiState, NetworkStatus, ReloadBehavior, SchemaResponse, SkillApiState,
    SkillDetailResponse, SkillListItem, SkillListResponse, StatusResponse, StatusState,
    StreamState, TailscaleStatus, TelegramUser, TelegramUserRole, UpdateConfigRequest,
    UpdateConfigResponse, UpdateSkillRequest, UpdateUserRequest, UserApiState, UserDetail,
    UserExport, UserListItem, UserListResponse, UserStats, ValidateConfigRequest,
    ValidateConfigResponse,
};
pub use auth::{
    auth_middleware, auth_router, require_role, AuthConfig, AuthError, AuthState, Claims,
    LoginRequest, LoginResponse, User, UserInfo, UserRole,
};
pub use config::DashboardConfig;
pub use server::DashboardServer;
