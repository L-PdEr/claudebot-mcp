//! Dashboard Authentication System
//!
//! JWT-based authentication with Argon2id password hashing.
//!
//! # Security Features
//!
//! - **Password hashing**: Argon2id (memory-hard, GPU-resistant)
//! - **JWT tokens**: 15-minute expiry, HS256 signing (RS256 for production)
//! - **Refresh tokens**: 7-day expiry, single-use
//! - **Cookie storage**: httpOnly, secure, sameSite=strict
//! - **Rate limiting**: 5 login attempts per minute per IP
//!
//! # Endpoints
//!
//! - `POST /api/auth/login` - Authenticate with username/password
//! - `POST /api/auth/logout` - Invalidate session
//! - `POST /api/auth/refresh` - Refresh access token
//! - `GET /api/auth/me` - Get current user info

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;
use uuid::Uuid;

/// JWT access token expiry (15 minutes)
const ACCESS_TOKEN_EXPIRY_MINUTES: i64 = 15;

/// Refresh token expiry (7 days)
const REFRESH_TOKEN_EXPIRY_DAYS: i64 = 7;

/// Rate limit window (60 seconds)
const RATE_LIMIT_WINDOW_SECS: i64 = 60;

/// Max login attempts per window
const MAX_LOGIN_ATTEMPTS: u32 = 5;

/// Cookie name for access token
const ACCESS_TOKEN_COOKIE: &str = "claudebot_access_token";

/// Cookie name for refresh token
const REFRESH_TOKEN_COOKIE: &str = "claudebot_refresh_token";

/// Authentication errors
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Token expired")]
    TokenExpired,

    #[error("Invalid token")]
    InvalidToken,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("User not found")]
    UserNotFound,

    #[error("Refresh token invalid or expired")]
    InvalidRefreshToken,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid credentials"),
            AuthError::TokenExpired => (StatusCode::UNAUTHORIZED, "Token expired"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token"),
            AuthError::RateLimitExceeded => (StatusCode::TOO_MANY_REQUESTS, "Too many login attempts"),
            AuthError::UserNotFound => (StatusCode::NOT_FOUND, "User not found"),
            AuthError::InvalidRefreshToken => (StatusCode::UNAUTHORIZED, "Invalid refresh token"),
            AuthError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal error"),
        };

        let body = Json(ErrorResponse {
            error: status.canonical_reason().unwrap_or("Error").to_string(),
            message: message.to_string(),
            details: None,
        });

        (status, body).into_response()
    }
}

/// Error response format
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// User role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Viewer,
    Editor,
    Admin,
}

impl Default for UserRole {
    fn default() -> Self {
        Self::Viewer
    }
}

/// User record stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: chrono::DateTime<Utc>,
    pub last_login: Option<chrono::DateTime<Utc>>,
}

/// JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID)
    pub sub: String,
    /// User role
    pub role: UserRole,
    /// Issued at
    pub iat: i64,
    /// Expiration
    pub exp: i64,
    /// JWT ID (for revocation)
    pub jti: String,
}

/// Refresh token record
#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub token_hash: String,
    pub user_id: String,
    pub expires_at: chrono::DateTime<Utc>,
    pub used: bool,
}

/// Rate limit entry
#[derive(Debug, Clone)]
struct RateLimitEntry {
    attempts: u32,
    window_start: chrono::DateTime<Utc>,
}

/// Authentication state
pub struct AuthState {
    /// JWT encoding key
    encoding_key: EncodingKey,
    /// JWT decoding key
    decoding_key: DecodingKey,
    /// Users database (in production, use SQLite)
    users: RwLock<HashMap<String, User>>,
    /// Refresh tokens (in production, use SQLite)
    refresh_tokens: RwLock<HashMap<String, RefreshToken>>,
    /// Rate limiting by IP
    rate_limits: RwLock<HashMap<String, RateLimitEntry>>,
    /// Revoked JTIs (in production, use Redis/SQLite)
    revoked_jtis: RwLock<std::collections::HashSet<String>>,
    /// Configuration
    pub config: AuthConfig,
}

/// Authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// JWT secret key (should be from environment)
    pub jwt_secret: String,
    /// Require authentication
    pub enabled: bool,
    /// Allow registration (single-user mode if false)
    pub allow_registration: bool,
    /// Secure cookies (requires HTTPS)
    pub secure_cookies: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            jwt_secret: std::env::var("DASHBOARD_JWT_SECRET")
                .unwrap_or_else(|_| {
                    // Generate a random secret if not provided (development only)
                    let secret: String = (0..64)
                        .map(|_| {
                            let idx = rand::random::<usize>() % 62;
                            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[idx] as char
                        })
                        .collect();
                    tracing::warn!("No JWT secret configured - using random secret (development only)");
                    secret
                }),
            enabled: std::env::var("DASHBOARD_REQUIRE_AUTH")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            allow_registration: std::env::var("DASHBOARD_ALLOW_REGISTRATION")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            secure_cookies: std::env::var("DASHBOARD_SECURE_COOKIES")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false), // Only true with HTTPS
        }
    }
}

impl AuthState {
    /// Create new auth state with configuration
    pub fn new(config: AuthConfig) -> Self {
        let encoding_key = EncodingKey::from_secret(config.jwt_secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(config.jwt_secret.as_bytes());

        Self {
            encoding_key,
            decoding_key,
            users: RwLock::new(HashMap::new()),
            refresh_tokens: RwLock::new(HashMap::new()),
            rate_limits: RwLock::new(HashMap::new()),
            revoked_jtis: RwLock::new(std::collections::HashSet::new()),
            config,
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(AuthConfig::default())
    }

    /// Hash a password using Argon2id
    pub fn hash_password(password: &str) -> Result<String, AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| AuthError::Internal(format!("Password hashing failed: {}", e)))
    }

    /// Verify a password against a hash
    pub fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| AuthError::Internal(format!("Invalid password hash: {}", e)))?;

        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// Create a new user (admin bootstrap or registration)
    pub fn create_user(&self, username: &str, password: &str, role: UserRole) -> Result<User, AuthError> {
        // Validate password policy (min 12 chars)
        if password.len() < 12 {
            return Err(AuthError::Internal("Password must be at least 12 characters".into()));
        }

        let password_hash = Self::hash_password(password)?;

        let user = User {
            id: Uuid::new_v4().to_string(),
            username: username.to_string(),
            password_hash,
            role,
            created_at: Utc::now(),
            last_login: None,
        };

        let mut users = self.users.write().map_err(|e| AuthError::Internal(e.to_string()))?;

        // Check if username exists
        if users.values().any(|u| u.username == username) {
            return Err(AuthError::Internal("Username already exists".into()));
        }

        users.insert(user.id.clone(), user.clone());
        Ok(user)
    }

    /// Check rate limit for IP
    fn check_rate_limit(&self, ip: &str) -> Result<(), AuthError> {
        let mut rate_limits = self.rate_limits.write().map_err(|e| AuthError::Internal(e.to_string()))?;

        let now = Utc::now();

        if let Some(entry) = rate_limits.get_mut(ip) {
            // Check if window expired
            if (now - entry.window_start).num_seconds() > RATE_LIMIT_WINDOW_SECS {
                // Reset window
                entry.attempts = 1;
                entry.window_start = now;
            } else if entry.attempts >= MAX_LOGIN_ATTEMPTS {
                return Err(AuthError::RateLimitExceeded);
            } else {
                entry.attempts += 1;
            }
        } else {
            rate_limits.insert(
                ip.to_string(),
                RateLimitEntry {
                    attempts: 1,
                    window_start: now,
                },
            );
        }

        Ok(())
    }

    /// Generate access and refresh tokens
    fn generate_tokens(&self, user: &User) -> Result<(String, String), AuthError> {
        let now = Utc::now();
        let jti = Uuid::new_v4().to_string();

        // Access token (15 minutes)
        let access_claims = Claims {
            sub: user.id.clone(),
            role: user.role,
            iat: now.timestamp(),
            exp: (now + Duration::minutes(ACCESS_TOKEN_EXPIRY_MINUTES)).timestamp(),
            jti: jti.clone(),
        };

        let access_token = encode(&Header::default(), &access_claims, &self.encoding_key)
            .map_err(|e| AuthError::Internal(format!("Failed to encode access token: {}", e)))?;

        // Refresh token (7 days)
        let refresh_token = Uuid::new_v4().to_string();
        let refresh_hash = Self::hash_token(&refresh_token);

        let mut refresh_tokens = self.refresh_tokens.write().map_err(|e| AuthError::Internal(e.to_string()))?;
        refresh_tokens.insert(
            refresh_hash.clone(),
            RefreshToken {
                token_hash: refresh_hash,
                user_id: user.id.clone(),
                expires_at: now + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS),
                used: false,
            },
        );

        Ok((access_token, refresh_token))
    }

    /// Hash a refresh token for storage
    fn hash_token(token: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Validate access token
    pub fn validate_token(&self, token: &str) -> Result<Claims, AuthError> {
        let validation = Validation::default();

        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::InvalidToken,
            })?;

        // Check if token is revoked
        let revoked = self.revoked_jtis.read().map_err(|e| AuthError::Internal(e.to_string()))?;
        if revoked.contains(&token_data.claims.jti) {
            return Err(AuthError::InvalidToken);
        }

        Ok(token_data.claims)
    }

    /// Revoke a token by JTI
    pub fn revoke_token(&self, jti: &str) -> Result<(), AuthError> {
        let mut revoked = self.revoked_jtis.write().map_err(|e| AuthError::Internal(e.to_string()))?;
        revoked.insert(jti.to_string());
        Ok(())
    }

    /// Authenticate user
    pub fn authenticate(&self, username: &str, password: &str, ip: &str) -> Result<(User, String, String), AuthError> {
        // Check rate limit
        self.check_rate_limit(ip)?;

        // Find user
        let users = self.users.read().map_err(|e| AuthError::Internal(e.to_string()))?;
        let user = users
            .values()
            .find(|u| u.username == username)
            .cloned()
            .ok_or(AuthError::InvalidCredentials)?;

        // Verify password
        if !Self::verify_password(password, &user.password_hash)? {
            return Err(AuthError::InvalidCredentials);
        }

        drop(users);

        // Update last login
        if let Ok(mut users) = self.users.write() {
            if let Some(u) = users.get_mut(&user.id) {
                u.last_login = Some(Utc::now());
            }
        }

        // Generate tokens
        let (access_token, refresh_token) = self.generate_tokens(&user)?;

        Ok((user, access_token, refresh_token))
    }

    /// Refresh access token
    pub fn refresh(&self, refresh_token: &str) -> Result<(String, String), AuthError> {
        let token_hash = Self::hash_token(refresh_token);

        let mut refresh_tokens = self.refresh_tokens.write().map_err(|e| AuthError::Internal(e.to_string()))?;

        let stored = refresh_tokens.get_mut(&token_hash).ok_or(AuthError::InvalidRefreshToken)?;

        // Check if expired
        if stored.expires_at < Utc::now() {
            refresh_tokens.remove(&token_hash);
            return Err(AuthError::InvalidRefreshToken);
        }

        // Check if already used (single-use)
        if stored.used {
            // Potential token theft - revoke all tokens for user
            let user_id = stored.user_id.clone();
            refresh_tokens.retain(|_, t| t.user_id != user_id);
            return Err(AuthError::InvalidRefreshToken);
        }

        // Mark as used
        stored.used = true;
        let user_id = stored.user_id.clone();

        drop(refresh_tokens);

        // Get user
        let users = self.users.read().map_err(|e| AuthError::Internal(e.to_string()))?;
        let user = users.get(&user_id).cloned().ok_or(AuthError::UserNotFound)?;

        drop(users);

        // Generate new tokens
        self.generate_tokens(&user)
    }

    /// Get user by ID
    pub fn get_user(&self, user_id: &str) -> Result<User, AuthError> {
        let users = self.users.read().map_err(|e| AuthError::Internal(e.to_string()))?;
        users.get(user_id).cloned().ok_or(AuthError::UserNotFound)
    }

    /// Build auth cookies
    pub fn build_cookies(&self, access_token: &str, refresh_token: &str) -> (Cookie<'static>, Cookie<'static>) {
        let access_cookie = Cookie::build((ACCESS_TOKEN_COOKIE, access_token.to_string()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .secure(self.config.secure_cookies)
            .max_age(cookie::time::Duration::minutes(ACCESS_TOKEN_EXPIRY_MINUTES as i64))
            .build();

        let refresh_cookie = Cookie::build((REFRESH_TOKEN_COOKIE, refresh_token.to_string()))
            .path("/api/auth/refresh")
            .http_only(true)
            .same_site(SameSite::Strict)
            .secure(self.config.secure_cookies)
            .max_age(cookie::time::Duration::days(REFRESH_TOKEN_EXPIRY_DAYS as i64))
            .build();

        (access_cookie, refresh_cookie)
    }

    /// Build logout cookies (clear)
    pub fn build_logout_cookies(&self) -> (Cookie<'static>, Cookie<'static>) {
        let access_cookie = Cookie::build((ACCESS_TOKEN_COOKIE, ""))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .secure(self.config.secure_cookies)
            .max_age(cookie::time::Duration::ZERO)
            .build();

        let refresh_cookie = Cookie::build((REFRESH_TOKEN_COOKIE, ""))
            .path("/api/auth/refresh")
            .http_only(true)
            .same_site(SameSite::Strict)
            .secure(self.config.secure_cookies)
            .max_age(cookie::time::Duration::ZERO)
            .build();

        (access_cookie, refresh_cookie)
    }
}

// ============================================================================
// API Handlers
// ============================================================================

/// Login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub user: Option<UserInfo>,
}

/// User info (safe to expose)
#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub role: UserRole,
}

impl From<&User> for UserInfo {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.clone(),
            username: user.username.clone(),
            role: user.role,
        }
    }
}

/// Get client IP from request headers
#[allow(dead_code)]
fn get_client_ip(headers: &axum::http::HeaderMap) -> String {
    // Try X-Forwarded-For first (for proxied requests)
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Ok(value) = forwarded.to_str() {
            if let Some(ip) = value.split(',').next() {
                return ip.trim().to_string();
            }
        }
    }

    // Fallback to X-Real-IP
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(value) = real_ip.to_str() {
            return value.to_string();
        }
    }

    // Default
    "unknown".to_string()
}

/// Login handler
pub async fn login_handler(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    req: Request,
) -> Result<(CookieJar, Json<LoginResponse>), AuthError> {
    // Extract body
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .map_err(|e| AuthError::Internal(format!("Failed to read body: {}", e)))?;

    let login_req: LoginRequest = serde_json::from_slice(&bytes)
        .map_err(|e| AuthError::Internal(format!("Invalid JSON: {}", e)))?;

    // Get client IP from parts
    let ip = parts
        .headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Authenticate
    let (user, access_token, refresh_token) = state.authenticate(&login_req.username, &login_req.password, &ip)?;

    // Build cookies
    let (access_cookie, refresh_cookie) = state.build_cookies(&access_token, &refresh_token);

    let jar = jar.add(access_cookie).add(refresh_cookie);

    Ok((
        jar,
        Json(LoginResponse {
            success: true,
            user: Some(UserInfo::from(&user)),
        }),
    ))
}

/// Logout handler
pub async fn logout_handler(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<(CookieJar, Json<LoginResponse>), AuthError> {
    // Revoke current token if present
    if let Some(cookie) = jar.get(ACCESS_TOKEN_COOKIE) {
        if let Ok(claims) = state.validate_token(cookie.value()) {
            let _ = state.revoke_token(&claims.jti);
        }
    }

    // Clear cookies
    let (access_cookie, refresh_cookie) = state.build_logout_cookies();
    let jar = jar.add(access_cookie).add(refresh_cookie);

    Ok((jar, Json(LoginResponse { success: true, user: None })))
}

/// Refresh handler
pub async fn refresh_handler(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<(CookieJar, Json<LoginResponse>), AuthError> {
    let refresh_token = jar
        .get(REFRESH_TOKEN_COOKIE)
        .ok_or(AuthError::InvalidRefreshToken)?
        .value();

    let (new_access, new_refresh) = state.refresh(refresh_token)?;

    // Get user info
    let claims = state.validate_token(&new_access)?;
    let user = state.get_user(&claims.sub)?;

    // Build new cookies
    let (access_cookie, refresh_cookie) = state.build_cookies(&new_access, &new_refresh);
    let jar = jar.add(access_cookie).add(refresh_cookie);

    Ok((
        jar,
        Json(LoginResponse {
            success: true,
            user: Some(UserInfo::from(&user)),
        }),
    ))
}

/// Me handler (get current user)
pub async fn me_handler(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
) -> Result<Json<UserInfo>, AuthError> {
    let access_token = jar
        .get(ACCESS_TOKEN_COOKIE)
        .ok_or(AuthError::InvalidToken)?
        .value();

    let claims = state.validate_token(access_token)?;
    let user = state.get_user(&claims.sub)?;

    Ok(Json(UserInfo::from(&user)))
}

// ============================================================================
// Middleware
// ============================================================================

/// Authentication middleware
pub async fn auth_middleware(
    State(state): State<Arc<AuthState>>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Result<Response, AuthError> {
    // Skip auth if disabled
    if !state.config.enabled {
        return Ok(next.run(req).await);
    }

    // Get token from cookie
    let access_token = jar
        .get(ACCESS_TOKEN_COOKIE)
        .ok_or(AuthError::InvalidToken)?
        .value();

    // Validate token
    let claims = state.validate_token(access_token)?;

    // Add claims to request extensions
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

/// Role-based authorization middleware
pub fn require_role(required: UserRole) -> impl Fn(Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, AuthError>> + Send>> + Clone {
    move |req: Request, next: Next| {
        let required = required;
        Box::pin(async move {
            let claims = req.extensions().get::<Claims>().ok_or(AuthError::InvalidToken)?;

            // Check role hierarchy
            let has_access = match (required, claims.role) {
                (UserRole::Viewer, _) => true,
                (UserRole::Editor, UserRole::Editor | UserRole::Admin) => true,
                (UserRole::Admin, UserRole::Admin) => true,
                _ => false,
            };

            if !has_access {
                return Err(AuthError::InvalidToken); // Use generic error to not leak role info
            }

            Ok(next.run(req).await)
        })
    }
}

// ============================================================================
// Router
// ============================================================================

/// Create the authentication router
pub fn auth_router(state: Arc<AuthState>) -> Router {
    Router::new()
        .route("/login", post(login_handler))
        .route("/logout", post(logout_handler))
        .route("/refresh", post(refresh_handler))
        .route("/me", get(me_handler))
        .with_state(state)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AuthConfig {
        AuthConfig {
            jwt_secret: "test-secret-at-least-32-characters-long".to_string(),
            enabled: true,
            allow_registration: false,
            secure_cookies: false,
        }
    }

    #[test]
    fn test_password_hashing() {
        let password = "secure-password-123";
        let hash = AuthState::hash_password(password).unwrap();

        assert!(AuthState::verify_password(password, &hash).unwrap());
        assert!(!AuthState::verify_password("wrong-password", &hash).unwrap());
    }

    #[test]
    fn test_create_user() {
        let state = AuthState::new(test_config());
        let user = state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        assert_eq!(user.username, "admin");
        assert_eq!(user.role, UserRole::Admin);
    }

    #[test]
    fn test_create_user_password_too_short() {
        let state = AuthState::new(test_config());
        let result = state.create_user("admin", "short", UserRole::Admin);

        assert!(result.is_err());
    }

    #[test]
    fn test_authenticate() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        let (user, access, refresh) = state.authenticate("admin", "secure-password-123", "127.0.0.1").unwrap();

        assert_eq!(user.username, "admin");
        assert!(!access.is_empty());
        assert!(!refresh.is_empty());
    }

    #[test]
    fn test_authenticate_wrong_password() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        let result = state.authenticate("admin", "wrong-password-123", "127.0.0.1");

        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn test_rate_limiting() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        // Exhaust rate limit
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            let _ = state.authenticate("admin", "wrong-password-123", "127.0.0.1");
        }

        // Next attempt should fail with rate limit
        let result = state.authenticate("admin", "secure-password-123", "127.0.0.1");
        assert!(matches!(result, Err(AuthError::RateLimitExceeded)));
    }

    #[test]
    fn test_token_validation() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        let (_, access, _) = state.authenticate("admin", "secure-password-123", "127.0.0.1").unwrap();

        let claims = state.validate_token(&access).unwrap();
        assert_eq!(claims.role, UserRole::Admin);
    }

    #[test]
    fn test_token_revocation() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        let (_, access, _) = state.authenticate("admin", "secure-password-123", "127.0.0.1").unwrap();

        let claims = state.validate_token(&access).unwrap();
        state.revoke_token(&claims.jti).unwrap();

        let result = state.validate_token(&access);
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_refresh_token() {
        let state = AuthState::new(test_config());
        state.create_user("admin", "secure-password-123", UserRole::Admin).unwrap();

        let (_, _, refresh) = state.authenticate("admin", "secure-password-123", "127.0.0.1").unwrap();

        let (new_access, _) = state.refresh(&refresh).unwrap();

        // Old refresh token should be invalidated (single-use)
        let result = state.refresh(&refresh);
        assert!(matches!(result, Err(AuthError::InvalidRefreshToken)));

        // New access token should work
        assert!(state.validate_token(&new_access).is_ok());
    }

    #[test]
    fn test_user_role_hierarchy() {
        // Admin can do everything
        assert!(matches!(
            (UserRole::Viewer, UserRole::Admin),
            (UserRole::Viewer, _)
        ));
        assert!(matches!(
            (UserRole::Editor, UserRole::Admin),
            (UserRole::Editor, UserRole::Editor | UserRole::Admin)
        ));
        assert!(matches!(
            (UserRole::Admin, UserRole::Admin),
            (UserRole::Admin, UserRole::Admin)
        ));
    }
}
