//! Intelligent Task Router
//!
//! Routes messages to appropriate handlers with model selection.
//! Supports both keyword-based routing and Ollama/Llama classification.

use once_cell::sync::Lazy;
use regex::Regex;
use tracing::debug;

/// Routing targets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Api,      // Claude API direct (quick answers)
    Backend,  // Rust/Axum code
    Frontend, // Vue/Nuxt code
    Codebase, // Full codebase operations
    Circle,   // Development Circle pipeline
}

impl Target {
    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Api => "api",
            Target::Backend => "backend",
            Target::Frontend => "frontend",
            Target::Codebase => "codebase",
            Target::Circle => "circle",
        }
    }
}

/// Model hints for cost optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelHint {
    /// Fast, cheap - factual Q&A, simple lookups ($0.25/M)
    Haiku,
    /// Balanced - implementation, analysis ($3/M)
    #[default]
    Sonnet,
    /// Deep reasoning - architecture, security ($15/M)
    Opus,
}

impl ModelHint {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelHint::Haiku => "haiku",
            ModelHint::Sonnet => "sonnet",
            ModelHint::Opus => "opus",
        }
    }
}

/// Routing result
#[derive(Debug, Clone)]
pub struct RouteResult {
    pub target: Target,
    pub model: ModelHint,
    pub reasoning: String,
    pub confidence: f32,
}

impl RouteResult {
    /// Check if this route requires code execution
    pub fn needs_code_execution(&self) -> bool {
        matches!(
            self.target,
            Target::Backend | Target::Frontend | Target::Codebase | Target::Circle
        )
    }
}

// Keyword sets
static BACKEND_KEYWORDS: &[&str] = &[
    "rust", "axum", "tokio", "cargo", "crate",
    "src/", "crates/", ".rs",
    "wasm", "grpc", "proto", "api", "endpoint", "handler",
    "database", "sql", "sqlx", "migration",
    "indicator", "tax", "trading", "decimal",
    "websocket", "ws", "async",
];

static FRONTEND_KEYWORDS: &[&str] = &[
    "vue", "nuxt", "typescript", "ts", "javascript", "js",
    "tailwind", "css", "scss", "html",
    "frontend/", "components/", "pages/", "composables/",
    ".vue", ".ts", ".tsx",
    "component", "page", "layout", "chart", "ui",
    "button", "modal", "form", "input", "dropdown",
    "pinia", "store", "state",
];

static CODE_KEYWORDS: &[&str] = &[
    "code", "implement", "fix", "bug", "debug", "error",
    "refactor", "test", "function", "class", "method",
    "commit", "git", "push", "pull", "merge", "branch",
    "build", "compile", "deploy", "run",
    "add", "remove", "update", "change", "modify",
    "create", "delete", "rename", "move",
];

static CIRCLE_KEYWORDS: &[&str] = &[
    "/circle", "circle", "development circle",
    "full review", "quality pipeline",
];

static OPUS_KEYWORDS: &[&str] = &[
    "opus", "gründlich", "thorough", "deep", "complex",
    "architecture", "security audit", "review carefully",
    "design", "plan", "strategy",
];

static HAIKU_KEYWORDS: &[&str] = &[
    "quick", "simple", "fast", "kurz", "schnell",
    "format", "lint", "typo", "what is", "explain",
    "define", "meaning",
];

static EXPLICIT_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)@(backend|frontend|codebase|api|circle)\b").unwrap()
});

/// Task router with keyword analysis
pub struct TaskRouter {
    /// Optional Ollama URL for Llama-based classification
    ollama_url: Option<String>,
}

impl TaskRouter {
    pub fn new(ollama_url: Option<String>) -> Self {
        Self { ollama_url }
    }

    /// Route a message to appropriate target and model
    pub fn route(&self, message: &str) -> RouteResult {
        let msg_lower = message.to_lowercase();

        // 1. Check explicit @target
        if let Some(result) = self.check_explicit(message) {
            return result;
        }

        // 2. Check /circle command
        if CIRCLE_KEYWORDS.iter().any(|kw| msg_lower.contains(kw)) {
            return RouteResult {
                target: Target::Circle,
                model: ModelHint::Opus,
                reasoning: "Development Circle requested".to_string(),
                confidence: 1.0,
            };
        }

        // 3. Keyword analysis
        let has_code = CODE_KEYWORDS.iter().any(|kw| msg_lower.contains(kw));
        let backend_score: usize = BACKEND_KEYWORDS
            .iter()
            .filter(|kw| msg_lower.contains(*kw))
            .count();
        let frontend_score: usize = FRONTEND_KEYWORDS
            .iter()
            .filter(|kw| msg_lower.contains(*kw))
            .count();

        let model = self.determine_model(&msg_lower);

        // Route based on scores
        if has_code || backend_score > 0 || frontend_score > 0 {
            let (target, reasoning) = if backend_score > frontend_score {
                (Target::Backend, format!("Backend keywords (score: {})", backend_score))
            } else if frontend_score > backend_score {
                (Target::Frontend, format!("Frontend keywords (score: {})", frontend_score))
            } else {
                (Target::Codebase, "Mixed code task".to_string())
            };

            return RouteResult {
                target,
                model,
                reasoning,
                confidence: 0.8,
            };
        }

        // 4. Default to API
        RouteResult {
            target: Target::Api,
            model,
            reasoning: "General question".to_string(),
            confidence: 0.6,
        }
    }

    /// Route with Llama classification (async, uses Ollama)
    pub async fn route_with_llama(&self, message: &str) -> RouteResult {
        // First try keyword routing
        let keyword_result = self.route(message);

        // If high confidence or no Ollama, return keyword result
        if keyword_result.confidence >= 0.9 || self.ollama_url.is_none() {
            return keyword_result;
        }

        // Try Llama classification
        match self.classify_with_llama(message).await {
            Ok(model) => {
                debug!("Llama classified as {:?}", model);
                RouteResult {
                    target: keyword_result.target,
                    model,
                    reasoning: format!("{} (Llama)", keyword_result.reasoning),
                    confidence: 0.95,
                }
            }
            Err(e) => {
                debug!("Llama classification failed: {}, using keyword routing", e);
                keyword_result
            }
        }
    }

    /// Classify complexity using Ollama/Llama
    async fn classify_with_llama(&self, message: &str) -> anyhow::Result<ModelHint> {
        let url = self.ollama_url.as_ref().ok_or_else(|| anyhow::anyhow!("No Ollama URL"))?;

        let prompt = format!(
            r#"Classify this query's complexity. Respond with exactly one word: SIMPLE, MODERATE, or COMPLEX

SIMPLE: Factual questions, definitions, lookups, formatting
MODERATE: Code implementation, analysis, debugging, explanations
COMPLEX: Architecture design, security audit, novel algorithms, deep reasoning

Query: {}

Classification:"#,
            message
        );

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/api/generate", url))
            .json(&serde_json::json!({
                "model": "llama3.2:3b",
                "prompt": prompt,
                "stream": false,
                "options": {
                    "num_predict": 10,
                    "temperature": 0.1
                }
            }))
            .send()
            .await?;

        let result: serde_json::Value = response.json().await?;
        let text = result["response"].as_str().unwrap_or("MODERATE");

        Ok(match text.trim().to_uppercase().as_str() {
            "SIMPLE" => ModelHint::Haiku,
            "COMPLEX" => ModelHint::Opus,
            _ => ModelHint::Sonnet,
        })
    }

    /// Check for explicit @target mention
    fn check_explicit(&self, message: &str) -> Option<RouteResult> {
        let captures = EXPLICIT_PATTERN.captures(message)?;
        let target_str = captures.get(1)?.as_str().to_lowercase();

        let target = match target_str.as_str() {
            "backend" => Target::Backend,
            "frontend" => Target::Frontend,
            "codebase" => Target::Codebase,
            "api" => Target::Api,
            "circle" => Target::Circle,
            _ => Target::Codebase,
        };

        let model = if target == Target::Circle {
            ModelHint::Opus
        } else {
            ModelHint::Sonnet
        };

        Some(RouteResult {
            target,
            model,
            reasoning: format!("Explicit @{}", target_str),
            confidence: 1.0,
        })
    }

    /// Determine model from keywords
    fn determine_model(&self, msg_lower: &str) -> ModelHint {
        if OPUS_KEYWORDS.iter().any(|kw| msg_lower.contains(kw)) {
            ModelHint::Opus
        } else if HAIKU_KEYWORDS.iter().any(|kw| msg_lower.contains(kw)) {
            ModelHint::Haiku
        } else {
            ModelHint::Sonnet
        }
    }
}

impl Default for TaskRouter {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explicit_target() {
        let router = TaskRouter::new(None);

        let result = router.route("@backend fix the RSI calculation");
        assert_eq!(result.target, Target::Backend);
        assert_eq!(result.confidence, 1.0);

        let result = router.route("@frontend update the chart");
        assert_eq!(result.target, Target::Frontend);
    }

    #[test]
    fn test_keyword_routing() {
        let router = TaskRouter::new(None);

        let result = router.route("Fix the Rust handler for the trading API");
        assert_eq!(result.target, Target::Backend);

        let result = router.route("Update the Vue component for the chart");
        assert_eq!(result.target, Target::Frontend);
    }

    #[test]
    fn test_circle_routing() {
        let router = TaskRouter::new(None);

        let result = router.route("/circle run quality pipeline");
        assert_eq!(result.target, Target::Circle);
        assert_eq!(result.model, ModelHint::Opus);
    }

    #[test]
    fn test_model_hints() {
        let router = TaskRouter::new(None);

        let result = router.route("Do a thorough security audit");
        assert_eq!(result.model, ModelHint::Opus);

        let result = router.route("Quick format check");
        assert_eq!(result.model, ModelHint::Haiku);
    }

    #[test]
    fn test_default_to_api() {
        let router = TaskRouter::new(None);

        let result = router.route("What is the weather?");
        assert_eq!(result.target, Target::Api);
    }
}
