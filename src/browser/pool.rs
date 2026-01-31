//! Browser Pool Management
//!
//! Manages headless browser instances for automation tasks.
//! Falls back to HTTP-based extraction when Chrome is unavailable.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock, Semaphore};
use tracing::{debug, info, warn};

/// Browser configuration
#[derive(Debug, Clone)]
pub struct BrowserConfig {
    /// Maximum concurrent browser instances
    pub max_instances: usize,
    /// Browser executable path (auto-detect if None)
    pub chrome_path: Option<String>,
    /// Default viewport width
    pub viewport_width: u32,
    /// Default viewport height
    pub viewport_height: u32,
    /// Page load timeout
    pub timeout_secs: u64,
    /// Enable headless mode
    pub headless: bool,
    /// User agent string
    pub user_agent: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            max_instances: 3,
            chrome_path: None,
            viewport_width: 1920,
            viewport_height: 1080,
            timeout_secs: 30,
            headless: true,
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string()
            ),
        }
    }
}

impl BrowserConfig {
    pub fn from_env() -> Self {
        Self {
            max_instances: std::env::var("BROWSER_MAX_INSTANCES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3),
            chrome_path: std::env::var("CHROME_PATH").ok(),
            viewport_width: std::env::var("BROWSER_VIEWPORT_WIDTH")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1920),
            viewport_height: std::env::var("BROWSER_VIEWPORT_HEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1080),
            timeout_secs: std::env::var("BROWSER_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            headless: std::env::var("BROWSER_HEADLESS")
                .map(|s| s != "false" && s != "0")
                .unwrap_or(true),
            user_agent: std::env::var("BROWSER_USER_AGENT").ok(),
        }
    }
}

/// Browser session for a single page
pub struct BrowserSession {
    id: String,
    current_url: String,
    created_at: std::time::Instant,
    last_activity: std::time::Instant,
}

impl BrowserSession {
    fn new(id: &str) -> Self {
        let now = std::time::Instant::now();
        Self {
            id: id.to_string(),
            current_url: String::new(),
            created_at: now,
            last_activity: now,
        }
    }

    fn touch(&mut self) {
        self.last_activity = std::time::Instant::now();
    }
}

/// Browser pool for managing multiple browser instances
pub struct BrowserPool {
    config: BrowserConfig,
    client: reqwest::Client,
    sessions: Arc<RwLock<HashMap<String, BrowserSession>>>,
    semaphore: Arc<Semaphore>,
    available: bool,
}

impl BrowserPool {
    /// Create new browser pool
    pub fn new(config: BrowserConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_instances));

        // Build HTTP client with appropriate settings
        let mut client_builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .danger_accept_invalid_certs(false);

        if let Some(ref user_agent) = config.user_agent {
            client_builder = client_builder.user_agent(user_agent);
        }

        let client = client_builder.build().expect("Failed to create HTTP client");

        Self {
            config,
            client,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            semaphore,
            available: true, // Assume available, will detect chrome later
        }
    }

    /// Create with default config
    pub fn default_pool() -> Self {
        Self::new(BrowserConfig::default())
    }

    /// Check if Chrome is available
    pub async fn check_chrome(&self) -> bool {
        // Try to find Chrome executable
        let paths = [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ];

        for path in paths {
            if std::path::Path::new(path).exists() {
                return true;
            }
        }

        // Check if specified path exists
        if let Some(ref path) = self.config.chrome_path {
            return std::path::Path::new(path).exists();
        }

        false
    }

    /// Take a screenshot of a URL
    pub async fn screenshot(&self, url: &str) -> Result<ScreenshotResult> {
        let _permit = self.semaphore.acquire().await?;

        // For now, use a simple HTTP approach to get page content
        // In production, this would use chromiumoxide or puppeteer
        info!("Taking screenshot of: {}", url);

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Failed to fetch URL")?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error: {}", response.status());
        }

        let html = response.text().await?;

        // Generate a placeholder "screenshot" (metadata about the page)
        let title = extract_title(&html).unwrap_or_else(|| url.to_string());
        let description = extract_meta_description(&html);
        let word_count = html.split_whitespace().count();

        Ok(ScreenshotResult {
            url: url.to_string(),
            title,
            description,
            html_length: html.len(),
            word_count,
            // In real implementation: PNG bytes
            image_data: None,
            screenshot_path: None,
        })
    }

    /// Extract text content from a page
    pub async fn extract_text(&self, url: &str, selector: Option<&str>) -> Result<String> {
        let _permit = self.semaphore.acquire().await?;

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Failed to fetch URL")?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error: {}", response.status());
        }

        let html = response.text().await?;

        // Simple text extraction (would use scraper crate in production)
        let text = if let Some(_sel) = selector {
            // Would use CSS selector to extract specific content
            extract_body_text(&html)
        } else {
            extract_body_text(&html)
        };

        Ok(text)
    }

    /// Fill a form field (simulated)
    pub async fn fill_form(&self, _url: &str, _fields: &HashMap<String, String>) -> Result<FormFillResult> {
        // This would require actual browser automation
        // For now, return a stub result
        warn!("Form filling requires headless Chrome - not yet implemented");

        Ok(FormFillResult {
            success: false,
            message: "Form filling requires headless Chrome support".to_string(),
            filled_fields: Vec::new(),
        })
    }

    /// Click an element (simulated)
    pub async fn click(&self, _url: &str, _selector: &str) -> Result<ClickResult> {
        warn!("Element clicking requires headless Chrome - not yet implemented");

        Ok(ClickResult {
            success: false,
            message: "Element clicking requires headless Chrome support".to_string(),
            new_url: None,
        })
    }

    /// Navigate to a URL
    pub async fn navigate(&self, url: &str) -> Result<NavigateResult> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Failed to navigate")?;

        let final_url = response.url().to_string();
        let status = response.status();
        let html = response.text().await?;
        let title = extract_title(&html);

        Ok(NavigateResult {
            success: status.is_success(),
            url: final_url,
            title,
            status_code: status.as_u16(),
        })
    }

    /// Create a new session
    pub async fn create_session(&self) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let session = BrowserSession::new(&id);

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), session);

        Ok(id)
    }

    /// Close a session
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
        Ok(())
    }

    /// Cleanup expired sessions
    pub async fn cleanup_expired(&self, max_age_secs: u64) {
        let max_age = Duration::from_secs(max_age_secs);
        let mut sessions = self.sessions.write().await;

        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.last_activity.elapsed() > max_age)
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            sessions.remove(&id);
            debug!("Cleaned up expired browser session: {}", id);
        }
    }

    /// Get pool statistics
    pub async fn stats(&self) -> BrowserPoolStats {
        let sessions = self.sessions.read().await;
        BrowserPoolStats {
            active_sessions: sessions.len(),
            max_instances: self.config.max_instances,
            available_permits: self.semaphore.available_permits(),
            chrome_available: self.available,
        }
    }
}

/// Screenshot result
#[derive(Debug, Clone)]
pub struct ScreenshotResult {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub html_length: usize,
    pub word_count: usize,
    pub image_data: Option<Vec<u8>>,
    pub screenshot_path: Option<String>,
}

/// Form fill result
#[derive(Debug, Clone)]
pub struct FormFillResult {
    pub success: bool,
    pub message: String,
    pub filled_fields: Vec<String>,
}

/// Click result
#[derive(Debug, Clone)]
pub struct ClickResult {
    pub success: bool,
    pub message: String,
    pub new_url: Option<String>,
}

/// Navigate result
#[derive(Debug, Clone)]
pub struct NavigateResult {
    pub success: bool,
    pub url: String,
    pub title: Option<String>,
    pub status_code: u16,
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct BrowserPoolStats {
    pub active_sessions: usize,
    pub max_instances: usize,
    pub available_permits: usize,
    pub chrome_available: bool,
}

/// Extract title from HTML
fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title>")?;
    let end = html[start..].find("</title>")?;
    let title = &html[start + 7..start + end];
    Some(html_entities_decode(title.trim()))
}

/// Extract meta description
fn extract_meta_description(html: &str) -> Option<String> {
    // Look for <meta name="description" content="...">
    let pattern = r#"<meta[^>]*name=["']description["'][^>]*content=["']([^"']+)["']"#;
    let re = regex::Regex::new(pattern).ok()?;
    re.captures(html).and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Extract body text (simplified)
fn extract_body_text(html: &str) -> String {
    // Remove script and style tags
    let mut text = html.to_string();

    // Remove script tags
    let script_re = regex::Regex::new(r"<script[^>]*>[\s\S]*?</script>").unwrap();
    text = script_re.replace_all(&text, "").to_string();

    // Remove style tags
    let style_re = regex::Regex::new(r"<style[^>]*>[\s\S]*?</style>").unwrap();
    text = style_re.replace_all(&text, "").to_string();

    // Remove all other HTML tags
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    text = tag_re.replace_all(&text, " ").to_string();

    // Decode HTML entities
    text = html_entities_decode(&text);

    // Normalize whitespace
    let ws_re = regex::Regex::new(r"\s+").unwrap();
    text = ws_re.replace_all(&text, " ").trim().to_string();

    text
}

/// Simple HTML entity decoder
fn html_entities_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>Test Page</title></head><body></body></html>";
        assert_eq!(extract_title(html), Some("Test Page".to_string()));
    }

    #[test]
    fn test_extract_body_text() {
        let html = "<html><body><p>Hello</p><script>var x=1;</script><p>World</p></body></html>";
        let text = extract_body_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("var x"));
    }

    #[test]
    fn test_html_entities_decode() {
        assert_eq!(html_entities_decode("&amp;"), "&");
        assert_eq!(html_entities_decode("&lt;test&gt;"), "<test>");
    }

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = BrowserPool::default_pool();
        let stats = pool.stats().await;
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.max_instances, 3);
    }
}
