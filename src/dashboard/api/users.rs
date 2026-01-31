//! User Management API
//!
//! REST API endpoints for managing Telegram users.
//!
//! # Features
//!
//! - List users with stats (messages, tokens, last active)
//! - Block/unblock users
//! - Adjust rate limits per user
//! - View conversation history
//! - Export user data (GDPR compliance)
//! - Delete user data (GDPR right to deletion)
//!
//! # Endpoints
//!
//! - `GET /api/users` - List all users with stats
//! - `GET /api/users/:id` - Get user details
//! - `PATCH /api/users/:id` - Update user (block/unblock, limits)
//! - `DELETE /api/users/:id` - Delete user and all data
//! - `GET /api/users/:id/conversations` - Get user's conversation history
//! - `GET /api/users/:id/export` - Export all user data (GDPR)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// User role in the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelegramUserRole {
    /// Basic user with standard limits
    User,
    /// Power user with higher limits
    PowerUser,
    /// Admin with unlimited access
    Admin,
}

impl Default for TelegramUserRole {
    fn default() -> Self {
        Self::User
    }
}

/// Telegram user record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUser {
    /// Telegram user ID
    pub user_id: i64,
    /// Telegram username (optional)
    pub username: Option<String>,
    /// First name
    pub first_name: String,
    /// Last name (optional)
    pub last_name: Option<String>,
    /// Whether user is allowed to use the bot
    pub allowed: bool,
    /// User role
    pub role: TelegramUserRole,
    /// Daily message limit
    pub daily_message_limit: Option<u32>,
    /// Daily token limit
    pub daily_token_limit: Option<i64>,
    /// Total messages sent
    pub total_messages: u64,
    /// Total tokens used
    pub total_tokens: u64,
    /// Last activity timestamp
    pub last_active: Option<DateTime<Utc>>,
    /// First seen timestamp
    pub created_at: DateTime<Utc>,
    /// Admin notes about user
    pub notes: Option<String>,
}

/// User statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStats {
    pub messages_today: u64,
    pub messages_week: u64,
    pub messages_month: u64,
    pub tokens_today: i64,
    pub tokens_month: i64,
    pub cost_today_usd: f64,
    pub cost_month_usd: f64,
    pub avg_response_time_ms: Option<u64>,
}

/// User with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDetail {
    #[serde(flatten)]
    pub user: TelegramUser,
    pub stats: UserStats,
}

/// Conversation item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationItem {
    pub id: String,
    pub user_message: String,
    pub bot_response: String,
    pub timestamp: DateTime<Utc>,
    pub tokens_used: i64,
    pub response_time_ms: u64,
}

/// User API state
pub struct UserApiState {
    /// Known users (in production, from SQLite)
    users: RwLock<HashMap<i64, TelegramUser>>,
    /// Blocked users
    blocked_users: RwLock<HashSet<i64>>,
    /// Conversations (in production, from SQLite)
    conversations: RwLock<HashMap<i64, Vec<ConversationItem>>>,
    /// Data directory for exports
    data_dir: PathBuf,
}

impl UserApiState {
    /// Create new user API state
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            users: RwLock::new(HashMap::new()),
            blocked_users: RwLock::new(HashSet::new()),
            conversations: RwLock::new(HashMap::new()),
            data_dir,
        }
    }

    /// Create with default paths
    pub fn with_defaults() -> Self {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claudebot")
            .join("user_exports");

        std::fs::create_dir_all(&data_dir).ok();

        Self::new(data_dir)
    }

    /// Register or update a user (called when user interacts with bot)
    pub async fn register_user(
        &self,
        user_id: i64,
        username: Option<String>,
        first_name: String,
        last_name: Option<String>,
    ) {
        let mut users = self.users.write().await;

        if let Some(user) = users.get_mut(&user_id) {
            // Update existing user
            user.username = username;
            user.first_name = first_name;
            user.last_name = last_name;
            user.last_active = Some(Utc::now());
            user.total_messages += 1;
        } else {
            // Create new user
            let blocked = self.blocked_users.read().await;
            let allowed = !blocked.contains(&user_id);
            drop(blocked);

            users.insert(
                user_id,
                TelegramUser {
                    user_id,
                    username,
                    first_name,
                    last_name,
                    allowed,
                    role: TelegramUserRole::User,
                    daily_message_limit: None,
                    daily_token_limit: None,
                    total_messages: 1,
                    total_tokens: 0,
                    last_active: Some(Utc::now()),
                    created_at: Utc::now(),
                    notes: None,
                },
            );
        }
    }

    /// Record token usage for a user
    pub async fn record_usage(&self, user_id: i64, tokens: i64) {
        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            user.total_tokens += tokens as u64;
            user.last_active = Some(Utc::now());
        }
    }

    /// Record a conversation
    pub async fn record_conversation(
        &self,
        user_id: i64,
        user_message: String,
        bot_response: String,
        tokens_used: i64,
        response_time_ms: u64,
    ) {
        let mut conversations = self.conversations.write().await;
        let user_convos = conversations.entry(user_id).or_insert_with(Vec::new);

        user_convos.push(ConversationItem {
            id: uuid::Uuid::new_v4().to_string(),
            user_message,
            bot_response,
            timestamp: Utc::now(),
            tokens_used,
            response_time_ms,
        });

        // Keep only last 1000 conversations per user
        if user_convos.len() > 1000 {
            user_convos.drain(0..user_convos.len() - 1000);
        }
    }

    /// Check if user is blocked
    pub async fn is_blocked(&self, user_id: i64) -> bool {
        let blocked = self.blocked_users.read().await;
        blocked.contains(&user_id)
    }

    /// Block a user
    pub async fn block_user(&self, user_id: i64) {
        let mut blocked = self.blocked_users.write().await;
        blocked.insert(user_id);

        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            user.allowed = false;
        }

        info!("Blocked user {}", user_id);
    }

    /// Unblock a user
    pub async fn unblock_user(&self, user_id: i64) {
        let mut blocked = self.blocked_users.write().await;
        blocked.remove(&user_id);

        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            user.allowed = true;
        }

        info!("Unblocked user {}", user_id);
    }

    /// Get user by ID
    pub async fn get_user(&self, user_id: i64) -> Option<TelegramUser> {
        let users = self.users.read().await;
        users.get(&user_id).cloned()
    }

    /// List all users
    pub async fn list_users(&self) -> Vec<TelegramUser> {
        let users = self.users.read().await;
        users.values().cloned().collect()
    }

    /// Get user conversations
    pub async fn get_conversations(&self, user_id: i64) -> Vec<ConversationItem> {
        let conversations = self.conversations.read().await;
        conversations.get(&user_id).cloned().unwrap_or_default()
    }

    /// Delete user and all data
    pub async fn delete_user(&self, user_id: i64) -> bool {
        let mut users = self.users.write().await;
        let removed = users.remove(&user_id).is_some();

        if removed {
            // Remove conversations
            let mut conversations = self.conversations.write().await;
            conversations.remove(&user_id);

            // Remove from blocked list
            let mut blocked = self.blocked_users.write().await;
            blocked.remove(&user_id);

            info!("Deleted user {} and all associated data", user_id);
        }

        removed
    }

    /// Export user data (GDPR)
    pub async fn export_user_data(&self, user_id: i64) -> Option<UserExport> {
        let users = self.users.read().await;
        let user = users.get(&user_id)?;

        let conversations = self.conversations.read().await;
        let user_conversations = conversations.get(&user_id).cloned().unwrap_or_default();

        Some(UserExport {
            user: user.clone(),
            conversations: user_conversations,
            exported_at: Utc::now(),
        })
    }

    /// Calculate user stats (mock implementation)
    fn calculate_stats(&self, user: &TelegramUser) -> UserStats {
        // In production, this would query the usage database
        UserStats {
            messages_today: 0,
            messages_week: 0,
            messages_month: user.total_messages,
            tokens_today: 0,
            tokens_month: user.total_tokens as i64,
            cost_today_usd: 0.0,
            cost_month_usd: (user.total_tokens as f64) * 0.00001, // Rough estimate
            avg_response_time_ms: None,
        }
    }
}

/// User export data (GDPR)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserExport {
    pub user: TelegramUser,
    pub conversations: Vec<ConversationItem>,
    pub exported_at: DateTime<Utc>,
}

// ============================================================================
// Response Types
// ============================================================================

/// User list response
#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserListItem>,
    pub total: usize,
    pub blocked_count: usize,
}

/// User list item (summary)
#[derive(Debug, Serialize)]
pub struct UserListItem {
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: String,
    pub allowed: bool,
    pub role: TelegramUserRole,
    pub total_messages: u64,
    pub total_tokens: u64,
    pub last_active: Option<DateTime<Utc>>,
}

impl From<&TelegramUser> for UserListItem {
    fn from(user: &TelegramUser) -> Self {
        Self {
            user_id: user.user_id,
            username: user.username.clone(),
            first_name: user.first_name.clone(),
            allowed: user.allowed,
            role: user.role,
            total_messages: user.total_messages,
            total_tokens: user.total_tokens,
            last_active: user.last_active,
        }
    }
}

/// Update user request
#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub allowed: Option<bool>,
    pub role: Option<TelegramUserRole>,
    pub daily_message_limit: Option<u32>,
    pub daily_token_limit: Option<i64>,
    pub notes: Option<String>,
}

/// Conversations response
#[derive(Debug, Serialize)]
pub struct ConversationsResponse {
    pub conversations: Vec<ConversationItem>,
    pub total: usize,
    pub has_more: bool,
}

/// Query parameters for user list
#[derive(Debug, Deserialize, Default)]
pub struct UserListQuery {
    /// Filter by allowed status
    pub allowed: Option<bool>,
    /// Filter by role
    pub role: Option<TelegramUserRole>,
    /// Search by username or name
    pub q: Option<String>,
    /// Sort by field
    pub sort: Option<String>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// Query parameters for conversations
#[derive(Debug, Deserialize, Default)]
pub struct ConversationsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct UserErrorResponse {
    pub error: String,
    pub message: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// List all users
/// GET /api/users
pub async fn list_users(
    State(state): State<Arc<UserApiState>>,
    Query(query): Query<UserListQuery>,
) -> impl IntoResponse {
    let all_users = state.list_users().await;
    let blocked = state.blocked_users.read().await;
    let blocked_count = blocked.len();
    drop(blocked);

    // Filter users
    let mut users: Vec<UserListItem> = all_users
        .iter()
        .filter(|u| {
            // Filter by allowed status
            if let Some(allowed) = query.allowed {
                if u.allowed != allowed {
                    return false;
                }
            }

            // Filter by role
            if let Some(ref role) = query.role {
                if u.role != *role {
                    return false;
                }
            }

            // Filter by search query
            if let Some(ref q) = query.q {
                let lower = q.to_lowercase();
                let matches_username = u
                    .username
                    .as_ref()
                    .map(|n| n.to_lowercase().contains(&lower))
                    .unwrap_or(false);
                let matches_name = u.first_name.to_lowercase().contains(&lower);
                let matches_id = u.user_id.to_string().contains(&lower);

                if !matches_username && !matches_name && !matches_id {
                    return false;
                }
            }

            true
        })
        .map(UserListItem::from)
        .collect();

    // Sort
    match query.sort.as_deref() {
        Some("messages") => users.sort_by(|a, b| b.total_messages.cmp(&a.total_messages)),
        Some("tokens") => users.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens)),
        Some("active") => users.sort_by(|a, b| b.last_active.cmp(&a.last_active)),
        _ => users.sort_by(|a, b| a.user_id.cmp(&b.user_id)),
    }

    let total = users.len();

    // Pagination
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(100);

    let users: Vec<UserListItem> = users.into_iter().skip(offset).take(limit).collect();

    Json(UserListResponse {
        users,
        total,
        blocked_count,
    })
}

/// Get user details
/// GET /api/users/:id
pub async fn get_user(
    State(state): State<Arc<UserApiState>>,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    match state.get_user(user_id).await {
        Some(user) => {
            let stats = state.calculate_stats(&user);
            Json(UserDetail { user, stats }).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(UserErrorResponse {
                error: "not_found".to_string(),
                message: format!("User {} not found", user_id),
            }),
        )
            .into_response(),
    }
}

/// Update user
/// PATCH /api/users/:id
pub async fn update_user(
    State(state): State<Arc<UserApiState>>,
    Path(user_id): Path<i64>,
    Json(req): Json<UpdateUserRequest>,
) -> impl IntoResponse {
    // Check if user exists
    let existing = state.get_user(user_id).await;
    if existing.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(UserErrorResponse {
                error: "not_found".to_string(),
                message: format!("User {} not found", user_id),
            }),
        )
            .into_response();
    }

    // Update allowed status (block/unblock)
    if let Some(allowed) = req.allowed {
        if allowed {
            state.unblock_user(user_id).await;
        } else {
            state.block_user(user_id).await;
        }
    }

    // Update other fields
    {
        let mut users = state.users.write().await;
        if let Some(user) = users.get_mut(&user_id) {
            if let Some(role) = req.role {
                user.role = role;
            }
            if let Some(limit) = req.daily_message_limit {
                user.daily_message_limit = Some(limit);
            }
            if let Some(limit) = req.daily_token_limit {
                user.daily_token_limit = Some(limit);
            }
            if let Some(notes) = req.notes {
                user.notes = if notes.is_empty() { None } else { Some(notes) };
            }
        }
    }

    // Return updated user
    match state.get_user(user_id).await {
        Some(user) => {
            let stats = state.calculate_stats(&user);
            info!("Updated user {}", user_id);
            Json(UserDetail { user, stats }).into_response()
        }
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(UserErrorResponse {
                error: "internal_error".to_string(),
                message: "Failed to retrieve updated user".to_string(),
            }),
        )
            .into_response(),
    }
}

/// Delete user and all data
/// DELETE /api/users/:id
pub async fn delete_user(
    State(state): State<Arc<UserApiState>>,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    if state.delete_user(user_id).await {
        Json(serde_json::json!({
            "success": true,
            "message": format!("User {} and all associated data deleted", user_id)
        }))
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(UserErrorResponse {
                error: "not_found".to_string(),
                message: format!("User {} not found", user_id),
            }),
        )
            .into_response()
    }
}

/// Get user's conversation history
/// GET /api/users/:id/conversations
pub async fn get_conversations(
    State(state): State<Arc<UserApiState>>,
    Path(user_id): Path<i64>,
    Query(query): Query<ConversationsQuery>,
) -> impl IntoResponse {
    // Check if user exists
    if state.get_user(user_id).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(UserErrorResponse {
                error: "not_found".to_string(),
                message: format!("User {} not found", user_id),
            }),
        )
            .into_response();
    }

    let all_conversations = state.get_conversations(user_id).await;
    let total = all_conversations.len();

    // Pagination (newest first)
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).min(100);

    let mut conversations = all_conversations;
    conversations.reverse(); // Newest first
    let conversations: Vec<ConversationItem> =
        conversations.into_iter().skip(offset).take(limit).collect();

    let has_more = offset + conversations.len() < total;

    Json(ConversationsResponse {
        conversations,
        total,
        has_more,
    })
    .into_response()
}

/// Export user data (GDPR)
/// GET /api/users/:id/export
pub async fn export_user_data(
    State(state): State<Arc<UserApiState>>,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    match state.export_user_data(user_id).await {
        Some(export) => {
            info!("Exported data for user {}", user_id);
            Json(export).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(UserErrorResponse {
                error: "not_found".to_string(),
                message: format!("User {} not found", user_id),
            }),
        )
            .into_response(),
    }
}

// ============================================================================
// Router
// ============================================================================

/// Create the users API router
pub fn users_router(state: Arc<UserApiState>) -> Router {
    Router::new()
        .route("/", get(list_users))
        .route("/{id}", get(get_user).patch(update_user).delete(delete_user))
        .route("/{id}/conversations", get(get_conversations))
        .route("/{id}/export", get(export_user_data))
        .with_state(state)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<UserApiState> {
        Arc::new(UserApiState::with_defaults())
    }

    async fn state_with_user() -> Arc<UserApiState> {
        let state = test_state();
        state
            .register_user(12345, Some("testuser".to_string()), "Test".to_string(), None)
            .await;
        state
    }

    #[tokio::test]
    async fn test_list_users_empty() {
        let state = test_state();
        let app = users_router(state);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["users"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_list_users_with_user() {
        let state = state_with_user().await;
        let app = users_router(state);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total"], 1);
        assert_eq!(json["users"][0]["user_id"], 12345);
    }

    #[tokio::test]
    async fn test_get_user() {
        let state = state_with_user().await;
        let app = users_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/12345")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["user_id"], 12345);
        assert_eq!(json["first_name"], "Test");
    }

    #[tokio::test]
    async fn test_get_nonexistent_user() {
        let state = test_state();
        let app = users_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/99999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_block_user() {
        let state = state_with_user().await;
        let app = users_router(state.clone());

        // Verify user is allowed initially
        assert!(!state.is_blocked(12345).await);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/12345")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"allowed": false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify user is now blocked
        assert!(state.is_blocked(12345).await);
    }

    #[tokio::test]
    async fn test_delete_user() {
        let state = state_with_user().await;
        let app = users_router(state.clone());

        // Verify user exists
        assert!(state.get_user(12345).await.is_some());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/12345")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify user is deleted
        assert!(state.get_user(12345).await.is_none());
    }

    #[tokio::test]
    async fn test_export_user_data() {
        let state = state_with_user().await;
        let app = users_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/12345/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["user"]["user_id"], 12345);
        assert!(json["conversations"].is_array());
        assert!(json["exported_at"].is_string());
    }

    #[tokio::test]
    async fn test_get_conversations() {
        let state = state_with_user().await;

        // Add a conversation
        state
            .record_conversation(
                12345,
                "Hello".to_string(),
                "Hi there!".to_string(),
                100,
                500,
            )
            .await;

        let app = users_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/12345/conversations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["total"], 1);
        assert_eq!(json["conversations"][0]["user_message"], "Hello");
    }
}
