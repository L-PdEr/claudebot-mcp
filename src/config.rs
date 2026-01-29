//! Configuration management

use anyhow::Result;
use std::path::PathBuf;

/// Server configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// Anthropic API key (optional - some tools require it)
    pub anthropic_api_key: Option<String>,

    /// Ollama URL for local Llama routing (optional)
    pub ollama_url: Option<String>,

    /// Redis URL for task coordination (optional)
    pub redis_url: Option<String>,

    /// SQLite database path for memory
    pub db_path: PathBuf,

    /// Workspace root for memory files
    pub workspace_path: PathBuf,

    /// Enable response caching
    pub cache_enabled: bool,

    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,

    /// Default model (haiku, sonnet, opus)
    pub default_model: String,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();

        let ollama_url = std::env::var("OLLAMA_URL").ok();
        let redis_url = std::env::var("REDIS_URL").ok();

        let db_path = std::env::var("CLAUDEBOT_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("claudebot")
                    .join("memory.db")
            });

        let workspace_path = std::env::var("CLAUDEBOT_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));

        let cache_enabled = std::env::var("CLAUDEBOT_CACHE_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        let cache_ttl_secs = std::env::var("CLAUDEBOT_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        let default_model = std::env::var("CLAUDEBOT_DEFAULT_MODEL")
            .unwrap_or_else(|_| "opus".to_string());

        Ok(Self {
            anthropic_api_key,
            ollama_url,
            redis_url,
            db_path,
            workspace_path,
            cache_enabled,
            cache_ttl_secs,
            default_model,
        })
    }
}

// Platform-specific dirs fallback
mod dirs {
    use std::path::PathBuf;

    pub fn data_local_dir() -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .ok()
                .or_else(|| {
                    std::env::var("HOME")
                        .map(|h| PathBuf::from(h).join(".local/share"))
                        .ok()
                })
        }

        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join("Library/Application Support"))
                .ok()
        }

        #[cfg(target_os = "windows")]
        {
            std::env::var("LOCALAPPDATA").map(PathBuf::from).ok()
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }
}
