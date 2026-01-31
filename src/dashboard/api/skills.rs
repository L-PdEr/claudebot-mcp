//! Skill Management API
//!
//! REST API endpoints for managing installed skills.
//!
//! # Endpoints
//!
//! - `GET /api/skills` - List all installed skills
//! - `POST /api/skills` - Install new skill from TOML or URL
//! - `GET /api/skills/:name` - Get skill details
//! - `PATCH /api/skills/:name` - Enable/disable skill
//! - `DELETE /api/skills/:name` - Uninstall skill
//! - `GET /api/skills/stats` - Get skill statistics

use crate::skills::{
    GeneratedSkill, InstalledSkill, SkillDefinition, SkillRegistry, SkillSource, SkillStats,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Skill API state
pub struct SkillApiState {
    /// Skill registry
    pub registry: Arc<RwLock<SkillRegistry>>,
}

impl SkillApiState {
    /// Create new skill API state with registry
    pub fn new(registry: Arc<RwLock<SkillRegistry>>) -> Self {
        Self { registry }
    }

    /// Create with default registry location
    pub fn with_defaults() -> Self {
        Self {
            registry: Arc::new(RwLock::new(SkillRegistry::default_location())),
        }
    }
}

/// Skill list item (summary for listing)
#[derive(Debug, Serialize)]
pub struct SkillListItem {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub usage_count: u64,
    pub success_rate: f64,
    pub last_used: Option<i64>,
    pub source: String,
    pub tags: Vec<String>,
}

impl From<&InstalledSkill> for SkillListItem {
    fn from(skill: &InstalledSkill) -> Self {
        Self {
            name: skill.definition.skill.name.clone(),
            version: skill.definition.skill.version.clone(),
            description: skill.definition.skill.description.clone(),
            enabled: skill.enabled,
            usage_count: skill.usage_count,
            success_rate: skill.success_rate(),
            last_used: skill.last_used,
            source: match &skill.source {
                SkillSource::Generated => "generated".to_string(),
                SkillSource::Imported(_) => "imported".to_string(),
                SkillSource::Hub(url) => format!("hub:{}", url),
                SkillSource::Builtin => "builtin".to_string(),
            },
            tags: skill.definition.skill.tags.clone(),
        }
    }
}

/// Skill detail response (full info)
#[derive(Debug, Serialize)]
pub struct SkillDetailResponse {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub usage_count: u64,
    pub success_count: u64,
    pub success_rate: f64,
    pub last_used: Option<i64>,
    pub installed_at: i64,
    pub source: String,
    pub tags: Vec<String>,
    pub parameters: serde_json::Value,
    pub execution_type: String,
}

impl From<&InstalledSkill> for SkillDetailResponse {
    fn from(skill: &InstalledSkill) -> Self {
        Self {
            name: skill.definition.skill.name.clone(),
            version: skill.definition.skill.version.clone(),
            description: skill.definition.skill.description.clone(),
            enabled: skill.enabled,
            usage_count: skill.usage_count,
            success_count: skill.success_count,
            success_rate: skill.success_rate(),
            last_used: skill.last_used,
            installed_at: skill.installed_at,
            source: match &skill.source {
                SkillSource::Generated => "generated".to_string(),
                SkillSource::Imported(path) => format!("imported:{}", path.display()),
                SkillSource::Hub(url) => format!("hub:{}", url),
                SkillSource::Builtin => "builtin".to_string(),
            },
            tags: skill.definition.skill.tags.clone(),
            parameters: serde_json::to_value(&skill.definition.parameters).unwrap_or_default(),
            execution_type: format!("{:?}", skill.definition.execution.exec_type),
        }
    }
}

/// List skills response
#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillListItem>,
    pub total: usize,
    pub stats: SkillStats,
}

/// Install skill request
#[derive(Debug, Deserialize)]
pub struct InstallSkillRequest {
    /// Install type: "toml" or "url"
    #[serde(rename = "type")]
    pub install_type: String,
    /// TOML content or URL
    pub content: String,
}

/// Install skill response
#[derive(Debug, Serialize)]
pub struct InstallSkillResponse {
    pub success: bool,
    pub skill: Option<SkillListItem>,
    pub error: Option<String>,
}

/// Update skill request
#[derive(Debug, Deserialize)]
pub struct UpdateSkillRequest {
    pub enabled: Option<bool>,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct SkillErrorResponse {
    pub error: String,
    pub message: String,
}

impl SkillErrorResponse {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

/// Query parameters for skill list
#[derive(Debug, Deserialize, Default)]
pub struct SkillListQuery {
    /// Search query
    pub q: Option<String>,
    /// Filter by enabled status
    pub enabled: Option<bool>,
    /// Filter by tag
    pub tag: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// List all skills
/// GET /api/skills
pub async fn list_skills(
    State(state): State<Arc<SkillApiState>>,
    Query(query): Query<SkillListQuery>,
) -> impl IntoResponse {
    let registry = state.registry.read().await;
    let all_skills = registry.list().await;
    let stats = registry.stats().await;
    drop(registry);

    // Filter skills based on query
    let mut skills: Vec<SkillListItem> = all_skills
        .iter()
        .filter(|s| {
            // Filter by enabled status
            if let Some(enabled) = query.enabled {
                if s.enabled != enabled {
                    return false;
                }
            }

            // Filter by tag
            if let Some(ref tag) = query.tag {
                if !s.definition.skill.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    return false;
                }
            }

            // Filter by search query
            if let Some(ref q) = query.q {
                let lower = q.to_lowercase();
                let matches_name = s.definition.skill.name.to_lowercase().contains(&lower);
                let matches_desc = s.definition.skill.description.to_lowercase().contains(&lower);
                let matches_tag = s
                    .definition
                    .skill
                    .tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&lower));

                if !matches_name && !matches_desc && !matches_tag {
                    return false;
                }
            }

            true
        })
        .map(SkillListItem::from)
        .collect();

    // Sort by name
    skills.sort_by(|a, b| a.name.cmp(&b.name));

    let total = skills.len();

    Json(SkillListResponse {
        skills,
        total,
        stats,
    })
}

/// Install a new skill
/// POST /api/skills
pub async fn install_skill(
    State(state): State<Arc<SkillApiState>>,
    Json(req): Json<InstallSkillRequest>,
) -> impl IntoResponse {
    let result = match req.install_type.as_str() {
        "toml" => install_from_toml(&state, &req.content).await,
        "url" => install_from_url(&state, &req.content).await,
        _ => Err(anyhow::anyhow!("Invalid install type: must be 'toml' or 'url'")),
    };

    match result {
        Ok(skill) => (
            StatusCode::CREATED,
            Json(InstallSkillResponse {
                success: true,
                skill: Some(SkillListItem::from(&skill)),
                error: None,
            }),
        )
            .into_response(),
        Err(e) => {
            warn!("Failed to install skill: {}", e);
            (
                StatusCode::BAD_REQUEST,
                Json(InstallSkillResponse {
                    success: false,
                    skill: None,
                    error: Some(e.to_string()),
                }),
            )
                .into_response()
        }
    }
}

/// Install skill from TOML content
async fn install_from_toml(
    state: &SkillApiState,
    toml_content: &str,
) -> anyhow::Result<InstalledSkill> {
    // Parse TOML
    let definition: SkillDefinition = toml::from_str(toml_content)
        .map_err(|e| anyhow::anyhow!("Invalid TOML: {}", e))?;

    // Validate
    definition.validate()?;

    // Create generated skill (will be marked as imported since we have TOML)
    let generated = GeneratedSkill {
        definition: definition.clone(),
        confidence: 1.0, // User-provided skill, full confidence
        reasoning: "Installed via dashboard API".to_string(),
        tests: vec![],
        needs_approval: false,
    };

    // Check for duplicates
    let registry = state.registry.read().await;
    if registry.get(&definition.skill.name).await.is_some() {
        return Err(anyhow::anyhow!(
            "Skill '{}' already exists",
            definition.skill.name
        ));
    }
    drop(registry);

    // Install
    let registry = state.registry.write().await;
    let name = registry.install(generated).await?;
    let skill = registry.get(&name).await.ok_or_else(|| {
        anyhow::anyhow!("Failed to retrieve installed skill")
    })?;

    info!("Installed skill '{}' from TOML", name);
    Ok(skill)
}

/// Install skill from URL
async fn install_from_url(state: &SkillApiState, url: &str) -> anyhow::Result<InstalledSkill> {
    // Validate URL - basic check without url crate
    if !url.starts_with("https://") {
        return Err(anyhow::anyhow!("Only HTTPS URLs are allowed"));
    }

    // Simple URL validation
    if url.len() < 12 || url.contains(' ') {
        return Err(anyhow::anyhow!("Invalid URL format"));
    }

    // Fetch content
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch URL: {}", e))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch URL: HTTP {}",
            response.status()
        ));
    }

    let content = response
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

    // Install from fetched TOML
    install_from_toml(state, &content).await
}

/// Get skill details
/// GET /api/skills/:name
pub async fn get_skill(
    State(state): State<Arc<SkillApiState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let registry = state.registry.read().await;

    match registry.get(&name).await {
        Some(skill) => Json(SkillDetailResponse::from(&skill)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(SkillErrorResponse::new("not_found", &format!("Skill '{}' not found", name))),
        )
            .into_response(),
    }
}

/// Update skill (enable/disable)
/// PATCH /api/skills/:name
pub async fn update_skill(
    State(state): State<Arc<SkillApiState>>,
    Path(name): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> impl IntoResponse {
    let registry = state.registry.write().await;

    // Check if skill exists
    if registry.get(&name).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(SkillErrorResponse::new("not_found", &format!("Skill '{}' not found", name))),
        )
            .into_response();
    }

    // Update enabled status if provided
    if let Some(enabled) = req.enabled {
        let result = if enabled {
            registry.enable(&name).await
        } else {
            registry.disable(&name).await
        };

        if let Err(e) = result {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SkillErrorResponse::new("update_failed", &e.to_string())),
            )
                .into_response();
        }

        info!("Skill '{}' {}abled", name, if enabled { "en" } else { "dis" });
    }

    // Return updated skill
    match registry.get(&name).await {
        Some(skill) => Json(SkillDetailResponse::from(&skill)).into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SkillErrorResponse::new("internal_error", "Failed to retrieve updated skill")),
        )
            .into_response(),
    }
}

/// Delete skill
/// DELETE /api/skills/:name
pub async fn delete_skill(
    State(state): State<Arc<SkillApiState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let registry = state.registry.write().await;

    // Check if skill exists
    if registry.get(&name).await.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(SkillErrorResponse::new("not_found", &format!("Skill '{}' not found", name))),
        )
            .into_response();
    }

    // Uninstall
    match registry.uninstall(&name).await {
        Ok(()) => {
            info!("Uninstalled skill '{}'", name);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "success": true, "message": format!("Skill '{}' uninstalled", name) })),
            )
                .into_response()
        }
        Err(e) => {
            warn!("Failed to uninstall skill '{}': {}", name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SkillErrorResponse::new("uninstall_failed", &e.to_string())),
            )
                .into_response()
        }
    }
}

/// Get skill statistics
/// GET /api/skills/stats
pub async fn get_skill_stats(State(state): State<Arc<SkillApiState>>) -> impl IntoResponse {
    let registry = state.registry.read().await;
    let stats = registry.stats().await;
    Json(stats)
}

// ============================================================================
// Router
// ============================================================================

/// Create the skills API router
pub fn skills_router(state: Arc<SkillApiState>) -> Router {
    Router::new()
        .route("/", get(list_skills).post(install_skill))
        .route("/stats", get(get_skill_stats))
        .route("/{name}", get(get_skill).patch(update_skill).delete(delete_skill))
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

    fn test_state() -> Arc<SkillApiState> {
        Arc::new(SkillApiState::with_defaults())
    }

    #[tokio::test]
    async fn test_list_skills_empty() {
        let state = test_state();
        let app = skills_router(state);

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

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["skills"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_get_nonexistent_skill() {
        let state = test_state();
        let app = skills_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_skill_stats() {
        let state = test_state();
        let app = skills_router(state);

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

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["total"].is_number());
        assert!(json["enabled"].is_number());
        assert!(json["avg_success_rate"].is_number());
    }

    #[tokio::test]
    async fn test_install_invalid_toml() {
        let state = test_state();
        let app = skills_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"type": "toml", "content": "invalid toml {"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["success"], false);
        assert!(json["error"].is_string());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_skill() {
        let state = test_state();
        let app = skills_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
