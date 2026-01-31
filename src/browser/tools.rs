//! Browser Automation Tools
//!
//! MCP-compatible tools for browser automation.

use super::pool::{BrowserPool, BrowserConfig};
use crate::agent::tools::{Tool, ToolSchema, ToolResult};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::info;

/// Global browser pool (lazy initialized)
static BROWSER_POOL: OnceCell<Arc<BrowserPool>> = OnceCell::const_new();

/// Get or create the browser pool
async fn get_pool() -> Arc<BrowserPool> {
    BROWSER_POOL
        .get_or_init(|| async {
            Arc::new(BrowserPool::new(BrowserConfig::from_env()))
        })
        .await
        .clone()
}

/// Screenshot tool - capture webpage as image/metadata
pub fn browser_screenshot_tool() -> Tool {
    let schema = ToolSchema::new(
        "browser_screenshot",
        "Take a screenshot of a webpage and extract its metadata",
    )
    .with_string_param("url", "URL to screenshot", true)
    .with_int_param("width", "Viewport width (default: 1920)", false)
    .with_int_param("height", "Viewport height (default: 1080)", false);

    Tool::new(schema, |params: Value| async move {
        let url = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing url parameter"))?;

        let pool = get_pool().await;

        match pool.screenshot(url).await {
            Ok(result) => {
                let output = format!(
                    "Screenshot captured:\n\
                    URL: {}\n\
                    Title: {}\n\
                    Description: {}\n\
                    HTML Size: {} bytes\n\
                    Word Count: {}",
                    result.url,
                    result.title,
                    result.description.unwrap_or_else(|| "N/A".to_string()),
                    result.html_length,
                    result.word_count
                );

                Ok(ToolResult::success("browser_screenshot", output))
            }
            Err(e) => Ok(ToolResult::error("browser_screenshot", e.to_string())),
        }
    })
}

/// Click tool - click an element on a page
pub fn browser_click_tool() -> Tool {
    let schema = ToolSchema::new(
        "browser_click",
        "Click an element on a webpage using CSS selector",
    )
    .with_string_param("url", "URL of the page", true)
    .with_string_param("selector", "CSS selector of element to click", true);

    Tool::new(schema, |params: Value| async move {
        let url = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing url parameter"))?;
        let selector = params["selector"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing selector parameter"))?;

        let pool = get_pool().await;

        match pool.click(url, selector).await {
            Ok(result) => {
                let output = if result.success {
                    format!(
                        "Clicked element '{}'\nNew URL: {}",
                        selector,
                        result.new_url.unwrap_or_else(|| url.to_string())
                    )
                } else {
                    format!("Click failed: {}", result.message)
                };

                Ok(ToolResult {
                    tool_name: "browser_click".to_string(),
                    success: result.success,
                    content: output,
                    data: None,
                    duration_ms: 0,
                })
            }
            Err(e) => Ok(ToolResult::error("browser_click", e.to_string())),
        }
    })
}

/// Fill tool - fill form fields
pub fn browser_fill_tool() -> Tool {
    let schema = ToolSchema::new(
        "browser_fill",
        "Fill form fields on a webpage",
    )
    .with_string_param("url", "URL of the page with form", true)
    .with_object_param("fields", "Object with field selectors and values", true);

    Tool::new(schema, |params: Value| async move {
        let url = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing url parameter"))?;
        let fields = params["fields"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Missing fields parameter"))?;

        let pool = get_pool().await;

        // Convert fields to HashMap
        let mut field_map = std::collections::HashMap::new();
        for (key, value) in fields {
            if let Some(v) = value.as_str() {
                field_map.insert(key.clone(), v.to_string());
            }
        }

        match pool.fill_form(url, &field_map).await {
            Ok(result) => {
                let output = if result.success {
                    format!(
                        "Filled {} fields: {}",
                        result.filled_fields.len(),
                        result.filled_fields.join(", ")
                    )
                } else {
                    format!("Form fill failed: {}", result.message)
                };

                Ok(ToolResult {
                    tool_name: "browser_fill".to_string(),
                    success: result.success,
                    content: output,
                    data: None,
                    duration_ms: 0,
                })
            }
            Err(e) => Ok(ToolResult::error("browser_fill", e.to_string())),
        }
    })
}

/// Extract tool - extract text content from a page
pub fn browser_extract_tool() -> Tool {
    let schema = ToolSchema::new(
        "browser_extract",
        "Extract text content from a webpage",
    )
    .with_string_param("url", "URL to extract content from", true)
    .with_string_param("selector", "CSS selector to extract (optional, extracts body if not provided)", false);

    Tool::new(schema, |params: Value| async move {
        let url = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing url parameter"))?;
        let selector = params["selector"].as_str();

        let pool = get_pool().await;

        match pool.extract_text(url, selector).await {
            Ok(text) => {
                // Truncate if too long
                let truncated = if text.len() > 5000 {
                    format!("{}...\n\n[Truncated: {} total chars]", &text[..5000], text.len())
                } else {
                    text
                };

                Ok(ToolResult::success("browser_extract", truncated))
            }
            Err(e) => Ok(ToolResult::error("browser_extract", e.to_string())),
        }
    })
}

/// Navigate tool - navigate to a URL and get page info
pub fn browser_navigate_tool() -> Tool {
    let schema = ToolSchema::new(
        "browser_navigate",
        "Navigate to a URL and return page information",
    )
    .with_string_param("url", "URL to navigate to", true);

    Tool::new(schema, |params: Value| async move {
        let url = params["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing url parameter"))?;

        let pool = get_pool().await;

        match pool.navigate(url).await {
            Ok(result) => {
                let title_display = result.title.clone().unwrap_or_else(|| "N/A".to_string());
                let output = format!(
                    "Navigated to: {}\n\
                    Title: {}\n\
                    Status: {}",
                    result.url,
                    title_display,
                    result.status_code
                );

                Ok(ToolResult {
                    tool_name: "browser_navigate".to_string(),
                    success: result.success,
                    content: output,
                    data: Some(serde_json::json!({
                        "url": result.url,
                        "title": result.title,
                        "status_code": result.status_code
                    })),
                    duration_ms: 0,
                })
            }
            Err(e) => Ok(ToolResult::error("browser_navigate", e.to_string())),
        }
    })
}

/// Register all browser tools with a registry
pub fn register_browser_tools(registry: &mut crate::agent::tools::ToolRegistry) {
    registry.register(browser_screenshot_tool());
    registry.register(browser_click_tool());
    registry.register(browser_fill_tool());
    registry.register(browser_extract_tool());
    registry.register(browser_navigate_tool());

    info!("Registered 5 browser automation tools");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screenshot_tool_schema() {
        let tool = browser_screenshot_tool();
        assert_eq!(tool.schema.name, "browser_screenshot");
        assert!(tool.schema.description.contains("screenshot"));
    }

    #[test]
    fn test_extract_tool_schema() {
        let tool = browser_extract_tool();
        assert_eq!(tool.schema.name, "browser_extract");
    }
}
