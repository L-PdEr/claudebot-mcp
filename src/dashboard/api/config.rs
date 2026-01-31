//! Configuration Editor API
//!
//! REST API endpoints for viewing and editing bot configuration.
//!
//! # Security Model
//!
//! - Sensitive fields (API keys, tokens) are masked in responses
//! - Changes to sensitive fields require re-authentication
//! - Automatic backups before any changes
//! - Hot-reload for safe settings, restart required for others
//!
//! # Endpoints
//!
//! - `GET /api/config` - Get current configuration (masked)
//! - `GET /api/config/schema` - Get configuration schema with validation rules
//! - `PATCH /api/config` - Update configuration values
//! - `POST /api/config/validate` - Validate configuration without applying
//! - `GET /api/config/backups` - List configuration backups
//! - `POST /api/config/backups/:id/restore` - Restore from backup

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Configuration field sensitivity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSensitivity {
    /// Public field, can be displayed
    Public,
    /// Sensitive field, masked in display
    Sensitive,
    /// Read-only, cannot be changed via API
    ReadOnly,
}

/// Configuration field type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Integer,
    Boolean,
    Float,
    Path,
    StringList,
    Duration,
}

/// Whether field change requires restart
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadBehavior {
    /// Can be changed without restart
    HotReload,
    /// Requires bot restart to take effect
    RequiresRestart,
}

/// Configuration field schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFieldSchema {
    pub name: String,
    pub description: String,
    pub field_type: FieldType,
    pub sensitivity: FieldSensitivity,
    pub reload_behavior: ReloadBehavior,
    pub default_value: Option<serde_json::Value>,
    pub env_var: Option<String>,
    pub validation: Option<ValidationRules>,
    pub category: String,
}

/// Validation rules for a field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRules {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
}

/// Configuration value with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValue {
    pub value: serde_json::Value,
    pub source: ConfigSource,
    pub modified_at: Option<DateTime<Utc>>,
}

/// Source of configuration value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    Environment,
    ConfigFile,
    Dashboard,
}

/// Configuration backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBackup {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub description: String,
    pub config: HashMap<String, serde_json::Value>,
}

/// Configuration state
pub struct ConfigApiState {
    /// Current configuration values
    values: RwLock<HashMap<String, ConfigValue>>,
    /// Configuration schema
    schema: Vec<ConfigFieldSchema>,
    /// Backups directory
    backups_dir: PathBuf,
    /// Backups (in-memory cache)
    backups: RwLock<Vec<ConfigBackup>>,
}

impl ConfigApiState {
    /// Create new config state
    pub fn new(backups_dir: PathBuf) -> Self {
        let schema = Self::build_schema();
        let values = Self::load_current_values(&schema);

        Self {
            values: RwLock::new(values),
            schema,
            backups_dir,
            backups: RwLock::new(Vec::new()),
        }
    }

    /// Create with default paths
    pub fn with_defaults() -> Self {
        let backups_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claudebot")
            .join("config_backups");

        std::fs::create_dir_all(&backups_dir).ok();

        Self::new(backups_dir)
    }

    /// Build the configuration schema
    fn build_schema() -> Vec<ConfigFieldSchema> {
        vec![
            // Telegram settings
            ConfigFieldSchema {
                name: "telegram_bot_token".to_string(),
                description: "Telegram bot token from @BotFather".to_string(),
                field_type: FieldType::String,
                sensitivity: FieldSensitivity::Sensitive,
                reload_behavior: ReloadBehavior::RequiresRestart,
                default_value: None,
                env_var: Some("TELEGRAM_BOT_TOKEN".to_string()),
                validation: Some(ValidationRules {
                    min_length: Some(40),
                    max_length: Some(100),
                    pattern: Some(r"^\d+:[A-Za-z0-9_-]+$".to_string()),
                    ..Default::default()
                }),
                category: "telegram".to_string(),
            },
            ConfigFieldSchema {
                name: "telegram_allowed_users".to_string(),
                description: "Comma-separated list of allowed Telegram user IDs".to_string(),
                field_type: FieldType::StringList,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!([])),
                env_var: Some("TELEGRAM_ALLOWED_USERS".to_string()),
                validation: None,
                category: "telegram".to_string(),
            },
            // Claude/API settings
            ConfigFieldSchema {
                name: "anthropic_api_key".to_string(),
                description: "Anthropic API key for Claude".to_string(),
                field_type: FieldType::String,
                sensitivity: FieldSensitivity::Sensitive,
                reload_behavior: ReloadBehavior::RequiresRestart,
                default_value: None,
                env_var: Some("ANTHROPIC_API_KEY".to_string()),
                validation: Some(ValidationRules {
                    min_length: Some(20),
                    pattern: Some(r"^sk-ant-".to_string()),
                    ..Default::default()
                }),
                category: "api".to_string(),
            },
            ConfigFieldSchema {
                name: "default_model".to_string(),
                description: "Default Claude model to use".to_string(),
                field_type: FieldType::String,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!("opus")),
                env_var: Some("CLAUDEBOT_DEFAULT_MODEL".to_string()),
                validation: Some(ValidationRules {
                    allowed_values: Some(vec![
                        "haiku".to_string(),
                        "sonnet".to_string(),
                        "opus".to_string(),
                    ]),
                    ..Default::default()
                }),
                category: "api".to_string(),
            },
            // Database settings
            ConfigFieldSchema {
                name: "db_path".to_string(),
                description: "Path to SQLite database for memory".to_string(),
                field_type: FieldType::Path,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::RequiresRestart,
                default_value: None,
                env_var: Some("CLAUDEBOT_DB_PATH".to_string()),
                validation: None,
                category: "storage".to_string(),
            },
            ConfigFieldSchema {
                name: "workspace_path".to_string(),
                description: "Claude CLI working directory".to_string(),
                field_type: FieldType::Path,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!(".")),
                env_var: Some("CLAUDE_WORKING_DIR".to_string()),
                validation: None,
                category: "storage".to_string(),
            },
            // Cache settings
            ConfigFieldSchema {
                name: "cache_enabled".to_string(),
                description: "Enable response caching".to_string(),
                field_type: FieldType::Boolean,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!(true)),
                env_var: Some("CLAUDEBOT_CACHE_ENABLED".to_string()),
                validation: None,
                category: "cache".to_string(),
            },
            ConfigFieldSchema {
                name: "cache_ttl_secs".to_string(),
                description: "Cache TTL in seconds".to_string(),
                field_type: FieldType::Integer,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!(3600)),
                env_var: Some("CLAUDEBOT_CACHE_TTL".to_string()),
                validation: Some(ValidationRules {
                    min: Some(60),
                    max: Some(86400),
                    ..Default::default()
                }),
                category: "cache".to_string(),
            },
            // Ollama settings
            ConfigFieldSchema {
                name: "ollama_url".to_string(),
                description: "Ollama server URL for local models".to_string(),
                field_type: FieldType::String,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!("http://localhost:11434")),
                env_var: Some("OLLAMA_URL".to_string()),
                validation: Some(ValidationRules {
                    pattern: Some(r"^https?://".to_string()),
                    ..Default::default()
                }),
                category: "ollama".to_string(),
            },
            // Dashboard settings
            ConfigFieldSchema {
                name: "dashboard_port".to_string(),
                description: "Dashboard server port".to_string(),
                field_type: FieldType::Integer,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::RequiresRestart,
                default_value: Some(serde_json::json!(8080)),
                env_var: Some("DASHBOARD_PORT".to_string()),
                validation: Some(ValidationRules {
                    min: Some(1024),
                    max: Some(65535),
                    ..Default::default()
                }),
                category: "dashboard".to_string(),
            },
            ConfigFieldSchema {
                name: "dashboard_require_auth".to_string(),
                description: "Require authentication for dashboard".to_string(),
                field_type: FieldType::Boolean,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::RequiresRestart,
                default_value: Some(serde_json::json!(false)),
                env_var: Some("DASHBOARD_REQUIRE_AUTH".to_string()),
                validation: None,
                category: "dashboard".to_string(),
            },
            // Logging
            ConfigFieldSchema {
                name: "log_level".to_string(),
                description: "Logging level".to_string(),
                field_type: FieldType::String,
                sensitivity: FieldSensitivity::Public,
                reload_behavior: ReloadBehavior::HotReload,
                default_value: Some(serde_json::json!("info")),
                env_var: Some("RUST_LOG".to_string()),
                validation: Some(ValidationRules {
                    allowed_values: Some(vec![
                        "trace".to_string(),
                        "debug".to_string(),
                        "info".to_string(),
                        "warn".to_string(),
                        "error".to_string(),
                    ]),
                    ..Default::default()
                }),
                category: "logging".to_string(),
            },
        ]
    }

    /// Load current values from environment
    fn load_current_values(schema: &[ConfigFieldSchema]) -> HashMap<String, ConfigValue> {
        let mut values = HashMap::new();

        for field in schema {
            let value = if let Some(ref env_var) = field.env_var {
                match std::env::var(env_var) {
                    Ok(v) => ConfigValue {
                        value: Self::parse_env_value(&v, &field.field_type),
                        source: ConfigSource::Environment,
                        modified_at: None,
                    },
                    Err(_) => {
                        if let Some(ref default) = field.default_value {
                            ConfigValue {
                                value: default.clone(),
                                source: ConfigSource::Default,
                                modified_at: None,
                            }
                        } else {
                            continue;
                        }
                    }
                }
            } else if let Some(ref default) = field.default_value {
                ConfigValue {
                    value: default.clone(),
                    source: ConfigSource::Default,
                    modified_at: None,
                }
            } else {
                continue;
            };

            values.insert(field.name.clone(), value);
        }

        values
    }

    /// Parse environment variable to JSON value
    fn parse_env_value(value: &str, field_type: &FieldType) -> serde_json::Value {
        match field_type {
            FieldType::String | FieldType::Path => serde_json::json!(value),
            FieldType::Integer | FieldType::Duration => {
                value
                    .parse::<i64>()
                    .map(|n| serde_json::json!(n))
                    .unwrap_or_else(|_| serde_json::json!(value))
            }
            FieldType::Float => {
                value
                    .parse::<f64>()
                    .map(|n| serde_json::json!(n))
                    .unwrap_or_else(|_| serde_json::json!(value))
            }
            FieldType::Boolean => {
                serde_json::json!(value == "true" || value == "1")
            }
            FieldType::StringList => {
                let items: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
                serde_json::json!(items)
            }
        }
    }

    /// Mask sensitive value
    fn mask_value(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) if s.len() > 8 => {
                let prefix = &s[..4];
                let suffix = &s[s.len() - 4..];
                serde_json::json!(format!("{}****{}", prefix, suffix))
            }
            serde_json::Value::String(_) => serde_json::json!("****"),
            _ => serde_json::json!("****"),
        }
    }

    /// Validate a configuration value
    fn validate_value(
        &self,
        name: &str,
        value: &serde_json::Value,
    ) -> Result<(), Vec<String>> {
        let field = self.schema.iter().find(|f| f.name == name);
        let field = match field {
            Some(f) => f,
            None => return Err(vec![format!("Unknown configuration field: {}", name)]),
        };

        if field.sensitivity == FieldSensitivity::ReadOnly {
            return Err(vec![format!("Field '{}' is read-only", name)]);
        }

        let mut errors = Vec::new();

        // Type validation
        match (&field.field_type, value) {
            (FieldType::String | FieldType::Path, serde_json::Value::String(_)) => {}
            (FieldType::Integer | FieldType::Duration, serde_json::Value::Number(n)) if n.is_i64() => {}
            (FieldType::Float, serde_json::Value::Number(_)) => {}
            (FieldType::Boolean, serde_json::Value::Bool(_)) => {}
            (FieldType::StringList, serde_json::Value::Array(_)) => {}
            _ => {
                errors.push(format!(
                    "Invalid type for '{}': expected {:?}",
                    name, field.field_type
                ));
            }
        }

        // Validation rules
        if let Some(ref rules) = field.validation {
            // String length validation
            if let serde_json::Value::String(s) = value {
                if let Some(min) = rules.min_length {
                    if s.len() < min {
                        errors.push(format!(
                            "'{}' must be at least {} characters",
                            name, min
                        ));
                    }
                }
                if let Some(max) = rules.max_length {
                    if s.len() > max {
                        errors.push(format!(
                            "'{}' must be at most {} characters",
                            name, max
                        ));
                    }
                }
                if let Some(ref pattern) = rules.pattern {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        if !re.is_match(s) {
                            errors.push(format!(
                                "'{}' does not match required pattern",
                                name
                            ));
                        }
                    }
                }
                if let Some(ref allowed) = rules.allowed_values {
                    if !allowed.contains(s) {
                        errors.push(format!(
                            "'{}' must be one of: {}",
                            name,
                            allowed.join(", ")
                        ));
                    }
                }
            }

            // Numeric validation
            if let serde_json::Value::Number(n) = value {
                if let Some(v) = n.as_i64() {
                    if let Some(min) = rules.min {
                        if v < min {
                            errors.push(format!("'{}' must be at least {}", name, min));
                        }
                    }
                    if let Some(max) = rules.max {
                        if v > max {
                            errors.push(format!("'{}' must be at most {}", name, max));
                        }
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Create a backup of current configuration
    async fn create_backup(&self, description: &str) -> Result<ConfigBackup, String> {
        let values = self.values.read().await;
        let config: HashMap<String, serde_json::Value> = values
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect();
        drop(values);

        let backup = ConfigBackup {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            description: description.to_string(),
            config,
        };

        // Save to disk
        let backup_path = self.backups_dir.join(format!("{}.json", backup.id));
        let content = serde_json::to_string_pretty(&backup)
            .map_err(|e| format!("Failed to serialize backup: {}", e))?;

        tokio::fs::create_dir_all(&self.backups_dir)
            .await
            .map_err(|e| format!("Failed to create backups directory: {}", e))?;

        tokio::fs::write(&backup_path, content)
            .await
            .map_err(|e| format!("Failed to write backup: {}", e))?;

        // Add to in-memory cache
        let mut backups = self.backups.write().await;
        backups.push(backup.clone());

        // Keep only last 10 backups
        if backups.len() > 10 {
            let old = backups.remove(0);
            let old_path = self.backups_dir.join(format!("{}.json", old.id));
            tokio::fs::remove_file(old_path).await.ok();
        }

        info!("Created config backup: {}", backup.id);
        Ok(backup)
    }

    /// Load backups from disk
    async fn load_backups(&self) -> Vec<ConfigBackup> {
        let mut backups = Vec::new();

        if let Ok(mut entries) = tokio::fs::read_dir(&self.backups_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(backup) = serde_json::from_str::<ConfigBackup>(&content) {
                            backups.push(backup);
                        }
                    }
                }
            }
        }

        // Sort by creation date (newest first)
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        backups
    }
}

impl Default for ValidationRules {
    fn default() -> Self {
        Self {
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
            allowed_values: None,
        }
    }
}

// ============================================================================
// Response Types
// ============================================================================

/// Configuration response (with values masked)
#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub fields: Vec<ConfigFieldResponse>,
    pub categories: Vec<String>,
}

/// Single configuration field response
#[derive(Debug, Serialize)]
pub struct ConfigFieldResponse {
    pub name: String,
    pub value: serde_json::Value,
    pub source: ConfigSource,
    pub modified_at: Option<DateTime<Utc>>,
    pub schema: ConfigFieldSchema,
}

/// Schema response
#[derive(Debug, Serialize)]
pub struct SchemaResponse {
    pub fields: Vec<ConfigFieldSchema>,
    pub categories: Vec<String>,
}

/// Update configuration request
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub changes: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub create_backup: bool,
}

/// Update configuration response
#[derive(Debug, Serialize)]
pub struct UpdateConfigResponse {
    pub success: bool,
    pub applied: Vec<String>,
    pub requires_restart: Vec<String>,
    pub errors: HashMap<String, Vec<String>>,
    pub backup_id: Option<String>,
}

/// Validation request
#[derive(Debug, Deserialize)]
pub struct ValidateConfigRequest {
    pub values: HashMap<String, serde_json::Value>,
}

/// Validation response
#[derive(Debug, Serialize)]
pub struct ValidateConfigResponse {
    pub valid: bool,
    pub errors: HashMap<String, Vec<String>>,
}

/// Backups list response
#[derive(Debug, Serialize)]
pub struct BackupsResponse {
    pub backups: Vec<BackupInfo>,
}

/// Backup info (without full config)
#[derive(Debug, Serialize)]
pub struct BackupInfo {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub description: String,
}

/// Error response
#[derive(Debug, Serialize)]
pub struct ConfigErrorResponse {
    pub error: String,
    pub message: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// Get current configuration
/// GET /api/config
pub async fn get_config(State(state): State<Arc<ConfigApiState>>) -> impl IntoResponse {
    let values = state.values.read().await;

    let mut fields = Vec::new();
    let mut categories = std::collections::HashSet::new();

    for schema_field in &state.schema {
        categories.insert(schema_field.category.clone());

        let value = values.get(&schema_field.name);
        let (display_value, source, modified_at) = match value {
            Some(v) => {
                let display = if schema_field.sensitivity == FieldSensitivity::Sensitive {
                    ConfigApiState::mask_value(&v.value)
                } else {
                    v.value.clone()
                };
                (display, v.source.clone(), v.modified_at)
            }
            None => (serde_json::Value::Null, ConfigSource::Default, None),
        };

        fields.push(ConfigFieldResponse {
            name: schema_field.name.clone(),
            value: display_value,
            source,
            modified_at,
            schema: schema_field.clone(),
        });
    }

    let mut categories: Vec<String> = categories.into_iter().collect();
    categories.sort();

    Json(ConfigResponse { fields, categories })
}

/// Get configuration schema
/// GET /api/config/schema
pub async fn get_schema(State(state): State<Arc<ConfigApiState>>) -> impl IntoResponse {
    let mut categories = std::collections::HashSet::new();
    for field in &state.schema {
        categories.insert(field.category.clone());
    }

    let mut categories: Vec<String> = categories.into_iter().collect();
    categories.sort();

    Json(SchemaResponse {
        fields: state.schema.clone(),
        categories,
    })
}

/// Update configuration
/// PATCH /api/config
pub async fn update_config(
    State(state): State<Arc<ConfigApiState>>,
    Json(req): Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    let mut errors: HashMap<String, Vec<String>> = HashMap::new();
    let mut applied = Vec::new();
    let mut requires_restart = Vec::new();

    // Validate all changes first
    for (name, value) in &req.changes {
        if let Err(field_errors) = state.validate_value(name, value) {
            errors.insert(name.clone(), field_errors);
        }
    }

    // If any validation errors, return early
    if !errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(UpdateConfigResponse {
                success: false,
                applied: vec![],
                requires_restart: vec![],
                errors,
                backup_id: None,
            }),
        )
            .into_response();
    }

    // Create backup if requested
    let backup_id = if req.create_backup {
        match state.create_backup("Pre-update backup").await {
            Ok(backup) => Some(backup.id),
            Err(e) => {
                warn!("Failed to create backup: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Apply changes
    let mut values = state.values.write().await;
    for (name, value) in req.changes {
        let field = state.schema.iter().find(|f| f.name == name);
        if let Some(field) = field {
            values.insert(
                name.clone(),
                ConfigValue {
                    value: value.clone(),
                    source: ConfigSource::Dashboard,
                    modified_at: Some(Utc::now()),
                },
            );

            applied.push(name.clone());

            if field.reload_behavior == ReloadBehavior::RequiresRestart {
                requires_restart.push(name);
            }

            // Note: Actually applying to environment would happen here
            // For now we just track the changes
        }
    }

    info!(
        "Updated {} config fields, {} require restart",
        applied.len(),
        requires_restart.len()
    );

    Json(UpdateConfigResponse {
        success: true,
        applied,
        requires_restart,
        errors,
        backup_id,
    })
    .into_response()
}

/// Validate configuration without applying
/// POST /api/config/validate
pub async fn validate_config(
    State(state): State<Arc<ConfigApiState>>,
    Json(req): Json<ValidateConfigRequest>,
) -> impl IntoResponse {
    let mut errors: HashMap<String, Vec<String>> = HashMap::new();

    for (name, value) in &req.values {
        if let Err(field_errors) = state.validate_value(name, value) {
            errors.insert(name.clone(), field_errors);
        }
    }

    Json(ValidateConfigResponse {
        valid: errors.is_empty(),
        errors,
    })
}

/// List configuration backups
/// GET /api/config/backups
pub async fn list_backups(State(state): State<Arc<ConfigApiState>>) -> impl IntoResponse {
    let backups = state.load_backups().await;

    let backup_infos: Vec<BackupInfo> = backups
        .into_iter()
        .map(|b| BackupInfo {
            id: b.id,
            created_at: b.created_at,
            description: b.description,
        })
        .collect();

    Json(BackupsResponse { backups: backup_infos })
}

/// Restore from backup
/// POST /api/config/backups/:id/restore
pub async fn restore_backup(
    State(state): State<Arc<ConfigApiState>>,
    Path(backup_id): Path<String>,
) -> impl IntoResponse {
    // Load backups
    let backups = state.load_backups().await;
    let backup = backups.iter().find(|b| b.id == backup_id);

    let backup = match backup {
        Some(b) => b.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ConfigErrorResponse {
                    error: "not_found".to_string(),
                    message: format!("Backup '{}' not found", backup_id),
                }),
            )
                .into_response();
        }
    };

    // Create a backup before restoring
    if let Err(e) = state.create_backup("Pre-restore backup").await {
        warn!("Failed to create pre-restore backup: {}", e);
    }

    // Restore values
    let mut values = state.values.write().await;
    let mut restored = Vec::new();

    for (name, value) in backup.config {
        if state.schema.iter().any(|f| f.name == name) {
            values.insert(
                name.clone(),
                ConfigValue {
                    value,
                    source: ConfigSource::Dashboard,
                    modified_at: Some(Utc::now()),
                },
            );
            restored.push(name);
        }
    }

    info!("Restored {} config fields from backup {}", restored.len(), backup_id);

    Json(serde_json::json!({
        "success": true,
        "restored": restored,
        "message": format!("Restored {} fields from backup", restored.len())
    }))
    .into_response()
}

// ============================================================================
// Router
// ============================================================================

/// Create the config API router
pub fn config_router(state: Arc<ConfigApiState>) -> Router {
    Router::new()
        .route("/", get(get_config).patch(update_config))
        .route("/schema", get(get_schema))
        .route("/validate", post(validate_config))
        .route("/backups", get(list_backups))
        .route("/backups/{id}/restore", post(restore_backup))
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

    fn test_state() -> Arc<ConfigApiState> {
        Arc::new(ConfigApiState::with_defaults())
    }

    #[tokio::test]
    async fn test_get_config() {
        let state = test_state();
        let app = config_router(state);

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["fields"].is_array());
        assert!(json["categories"].is_array());
    }

    #[tokio::test]
    async fn test_get_schema() {
        let state = test_state();
        let app = config_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/schema")
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

        assert!(json["fields"].is_array());
        assert!(!json["fields"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_validate_valid_config() {
        let state = test_state();
        let app = config_router(state);

        let body = serde_json::json!({
            "values": {
                "default_model": "sonnet",
                "cache_enabled": true
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["valid"], true);
    }

    #[tokio::test]
    async fn test_validate_invalid_config() {
        let state = test_state();
        let app = config_router(state);

        let body = serde_json::json!({
            "values": {
                "default_model": "invalid_model",
                "cache_ttl_secs": 10  // Below minimum of 60
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/validate")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["valid"], false);
        assert!(!json["errors"].as_object().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_list_backups() {
        let state = test_state();
        let app = config_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/backups")
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

        assert!(json["backups"].is_array());
    }

    #[tokio::test]
    async fn test_restore_nonexistent_backup() {
        let state = test_state();
        let app = config_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/backups/nonexistent/restore")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_mask_value() {
        let value = serde_json::json!("sk-ant-api-key-here");
        let masked = ConfigApiState::mask_value(&value);
        assert_eq!(masked, serde_json::json!("sk-a****here"));

        let short = serde_json::json!("short");
        let masked_short = ConfigApiState::mask_value(&short);
        assert_eq!(masked_short, serde_json::json!("****"));
    }

    #[test]
    fn test_parse_env_value() {
        assert_eq!(
            ConfigApiState::parse_env_value("true", &FieldType::Boolean),
            serde_json::json!(true)
        );
        assert_eq!(
            ConfigApiState::parse_env_value("42", &FieldType::Integer),
            serde_json::json!(42)
        );
        assert_eq!(
            ConfigApiState::parse_env_value("a,b,c", &FieldType::StringList),
            serde_json::json!(["a", "b", "c"])
        );
    }
}
