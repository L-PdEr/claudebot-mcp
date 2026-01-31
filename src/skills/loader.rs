//! Skill Loader
//!
//! Loads skills from various sources: local files, URLs, and skill hubs.

use super::types::*;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Skill loader for importing from various sources
pub struct SkillLoader {
    client: reqwest::Client,
    cache_dir: PathBuf,
}

impl SkillLoader {
    /// Create new loader
    pub fn new() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claudebot")
            .join("skill-cache");

        std::fs::create_dir_all(&cache_dir).ok();

        Self {
            client: reqwest::Client::new(),
            cache_dir,
        }
    }

    /// Load skill from a path or URL
    pub async fn load(&self, source: &str) -> Result<SkillDefinition> {
        if source.starts_with("http://") || source.starts_with("https://") {
            self.load_from_url(source).await
        } else if source.starts_with("hub:") {
            let skill_name = source.strip_prefix("hub:").unwrap();
            self.load_from_hub(skill_name).await
        } else {
            self.load_from_file(Path::new(source)).await
        }
    }

    /// Load from local file
    pub async fn load_from_file(&self, path: &Path) -> Result<SkillDefinition> {
        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read skill file")?;

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match extension {
            "toml" => {
                toml::from_str(&content).context("Failed to parse TOML")
            }
            "json" => {
                serde_json::from_str(&content).context("Failed to parse JSON")
            }
            "yaml" | "yml" => {
                // Would need serde_yaml dependency
                anyhow::bail!("YAML not supported yet")
            }
            _ => {
                // Try TOML first, then JSON
                toml::from_str(&content)
                    .or_else(|_| serde_json::from_str(&content))
                    .context("Failed to parse skill file")
            }
        }
    }

    /// Load from URL
    pub async fn load_from_url(&self, url: &str) -> Result<SkillDefinition> {
        let response = self
            .client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .context("Failed to fetch skill from URL")?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error: {}", response.status());
        }

        let content = response.text().await?;

        // Determine format from URL or content
        if url.ends_with(".json") || content.trim().starts_with('{') {
            serde_json::from_str(&content).context("Failed to parse JSON")
        } else {
            toml::from_str(&content).context("Failed to parse TOML")
        }
    }

    /// Load from skill hub (GitHub-based)
    pub async fn load_from_hub(&self, skill_name: &str) -> Result<SkillDefinition> {
        // Hub format: skill_name or author/skill_name
        let (author, name) = if skill_name.contains('/') {
            let parts: Vec<&str> = skill_name.splitn(2, '/').collect();
            (parts[0], parts[1])
        } else {
            ("claudebot-skills", skill_name)
        };

        // Try different hub URLs in order
        let urls = [
            // GitHub raw content
            format!(
                "https://raw.githubusercontent.com/{}/{}/main/skill.toml",
                author, name
            ),
            // Alternative path
            format!(
                "https://raw.githubusercontent.com/{}/{}/main/{}.toml",
                author, name, name
            ),
            // Skills repository
            format!(
                "https://raw.githubusercontent.com/claudebot/skills/main/{}/{}.toml",
                author, name
            ),
        ];

        for url in &urls {
            match self.load_from_url(url).await {
                Ok(skill) => {
                    info!("Loaded skill '{}' from hub", skill_name);
                    return Ok(skill);
                }
                Err(e) => {
                    debug!("Failed to load from {}: {}", url, e);
                    continue;
                }
            }
        }

        anyhow::bail!("Skill '{}' not found in hub", skill_name)
    }

    /// Search hub for skills
    pub async fn search_hub(&self, query: &str) -> Result<Vec<HubSkillInfo>> {
        // This would integrate with a skill registry API
        // For now, return empty
        warn!("Hub search not yet implemented");
        Ok(Vec::new())
    }

    /// Get cached skill path
    fn cached_path(&self, name: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.toml", name))
    }

    /// Check if skill is cached
    pub fn is_cached(&self, name: &str) -> bool {
        self.cached_path(name).exists()
    }

    /// Load from cache
    pub async fn load_cached(&self, name: &str) -> Result<SkillDefinition> {
        self.load_from_file(&self.cached_path(name)).await
    }

    /// Save to cache
    pub async fn cache(&self, skill: &SkillDefinition) -> Result<PathBuf> {
        let path = self.cached_path(&skill.skill.name);
        let content = toml::to_string_pretty(skill)?;
        tokio::fs::write(&path, content).await?;
        Ok(path)
    }

    /// Clear cache
    pub async fn clear_cache(&self) -> Result<usize> {
        let mut count = 0;
        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().map(|e| e == "toml").unwrap_or(false) {
                tokio::fs::remove_file(entry.path()).await?;
                count += 1;
            }
        }

        Ok(count)
    }
}

impl Default for SkillLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Hub skill info (from search)
#[derive(Debug, Clone)]
pub struct HubSkillInfo {
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub downloads: u64,
    pub stars: u32,
    pub url: String,
}


#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_load_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.toml");

        let content = r#"
[skill]
name = "test_skill"
version = "1.0.0"
description = "A test skill"

[execution]
type = "shell"
command = "echo hello"
"#;

        tokio::fs::write(&file_path, content).await.unwrap();

        let loader = SkillLoader::new();
        let skill = loader.load_from_file(&file_path).await.unwrap();

        assert_eq!(skill.skill.name, "test_skill");
        assert_eq!(skill.skill.version, "1.0.0");
    }

    #[test]
    fn test_hub_name_parsing() {
        // Test parsing of hub skill names
        let full = "author/skill_name";
        let parts: Vec<&str> = full.splitn(2, '/').collect();
        assert_eq!(parts[0], "author");
        assert_eq!(parts[1], "skill_name");

        let simple = "skill_name";
        let has_slash = simple.contains('/');
        assert!(!has_slash);
    }
}
