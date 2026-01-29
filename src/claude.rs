//! Claude API Client
//!
//! Anthropic Claude API client with prompt caching support.
//! Uses cache_control: ephemeral for 90% cost reduction on static context.

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Claude API client
#[derive(Clone)]
pub struct ClaudeClient {
    client: Client,
    api_key: Option<String>,
}

impl ClaudeClient {
    /// Check if API key is configured
    pub fn is_available(&self) -> bool {
        self.api_key.is_some()
    }
}

/// System message block with optional cache control
#[derive(Debug, Serialize)]
struct SystemBlock {
    r#type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize)]
struct CacheControl {
    r#type: String,
}

/// Message in conversation
#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

/// API request
#[derive(Debug, Serialize)]
struct MessageRequest {
    model: String,
    max_tokens: usize,
    system: Vec<SystemBlock>,
    messages: Vec<Message>,
}

/// API response
#[derive(Debug, Deserialize)]
struct MessageResponse {
    content: Vec<ContentBlock>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    r#type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    input_tokens: usize,
    output_tokens: usize,
    #[serde(default)]
    cache_read_input_tokens: usize,
    #[serde(default)]
    cache_creation_input_tokens: usize,
}

/// Completion result with usage stats
#[derive(Debug, Clone)]
pub struct CompleteResult {
    pub content: String,
    pub model: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_read_tokens: usize,
    pub cache_write_tokens: usize,
}

impl CompleteResult {
    /// Calculate cost in USD (approximate)
    pub fn estimated_cost(&self) -> f64 {
        let (input_price, output_price) = match self.model.as_str() {
            m if m.contains("haiku") => (0.25, 1.25),   // per million
            m if m.contains("opus") => (15.0, 75.0),    // per million
            _ => (3.0, 15.0),                            // sonnet default
        };

        // Cached reads are 90% cheaper
        let cached_input_cost = (self.cache_read_tokens as f64 / 1_000_000.0) * input_price * 0.1;
        let uncached_input_cost = ((self.input_tokens - self.cache_read_tokens) as f64 / 1_000_000.0) * input_price;
        let output_cost = (self.output_tokens as f64 / 1_000_000.0) * output_price;

        cached_input_cost + uncached_input_cost + output_cost
    }

    /// Cache efficiency (0-100%)
    pub fn cache_efficiency(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        (self.cache_read_tokens as f64 / self.input_tokens as f64) * 100.0
    }
}

impl ClaudeClient {
    pub fn new(api_key: Option<&str>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.map(|s| s.to_string()),
        }
    }

    /// Create from config
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::new(config.anthropic_api_key.as_deref())
    }

    /// Simple chat - for quick interactions without caching
    pub async fn chat(&self, prompt: &str) -> Result<String> {
        let result = self.complete(
            prompt,
            "You are a helpful assistant.",
            None,
            4096,
            "sonnet",
        ).await?;
        Ok(result.content)
    }

    /// Get model ID from hint
    fn model_id(model: &str) -> &'static str {
        match model.to_lowercase().as_str() {
            "haiku" => "claude-3-5-haiku-20241022",
            "opus" => "claude-3-opus-20240229",
            _ => "claude-sonnet-4-20250514",
        }
    }

    /// Complete with prompt caching
    ///
    /// # Arguments
    /// * `prompt` - User message
    /// * `static_context` - Cached system context (SOUL, standards, domain)
    /// * `session_context` - Per-session context (cached separately)
    /// * `max_tokens` - Max response tokens
    /// * `model` - Model hint (haiku, sonnet, opus)
    pub async fn complete(
        &self,
        prompt: &str,
        static_context: &str,
        session_context: Option<&str>,
        max_tokens: usize,
        model: &str,
    ) -> Result<CompleteResult> {
        self.complete_with_history(prompt, static_context, session_context, max_tokens, model, &[])
            .await
    }

    /// Complete with conversation history
    pub async fn complete_with_history(
        &self,
        prompt: &str,
        static_context: &str,
        session_context: Option<&str>,
        max_tokens: usize,
        model: &str,
        history: &[(String, String)],
    ) -> Result<CompleteResult> {
        let model_id = Self::model_id(model);

        // Build system blocks with caching
        let mut system = vec![
            // Static context - always cached
            SystemBlock {
                r#type: "text".to_string(),
                text: static_context.to_string(),
                cache_control: Some(CacheControl {
                    r#type: "ephemeral".to_string(),
                }),
            },
        ];

        // Session context - cached separately
        if let Some(ctx) = session_context {
            if !ctx.is_empty() {
                system.push(SystemBlock {
                    r#type: "text".to_string(),
                    text: ctx.to_string(),
                    cache_control: Some(CacheControl {
                        r#type: "ephemeral".to_string(),
                    }),
                });
            }
        }

        // Build messages
        let mut messages: Vec<Message> = history
            .iter()
            .map(|(role, content)| Message {
                role: role.clone(),
                content: content.clone(),
            })
            .collect();

        messages.push(Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        });

        let request = MessageRequest {
            model: model_id.to_string(),
            max_tokens,
            system,
            messages,
        };

        debug!("Calling Claude API: model={}, prompt_len={}", model_id, prompt.len());

        let api_key = self.api_key.as_ref()
            .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY not set - Claude API tools unavailable"))?;

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("Claude API error {}: {}", status, text);
        }

        let result: MessageResponse = response.json().await?;

        let content = result
            .content
            .into_iter()
            .filter_map(|b| if b.r#type == "text" { b.text } else { None })
            .collect::<Vec<_>>()
            .join("\n");

        let complete_result = CompleteResult {
            content,
            model: model_id.to_string(),
            input_tokens: result.usage.input_tokens,
            output_tokens: result.usage.output_tokens,
            cache_read_tokens: result.usage.cache_read_input_tokens,
            cache_write_tokens: result.usage.cache_creation_input_tokens,
        };

        info!(
            "Claude response: model={}, in={}, out={}, cache_read={}, cache_write={}, efficiency={:.1}%",
            model_id,
            complete_result.input_tokens,
            complete_result.output_tokens,
            complete_result.cache_read_tokens,
            complete_result.cache_write_tokens,
            complete_result.cache_efficiency()
        );

        Ok(complete_result)
    }
}
