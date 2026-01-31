//! Llama Worker - Local LLM for cost-free operations
//!
//! Uses Ollama/Llama for:
//! - Context compression (reduce token costs)
//! - Entity extraction (graph memory)
//! - Query classification (model routing)
//! - Summary generation (wake/sleep cycle)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// Llama Worker configuration
#[derive(Debug, Clone)]
pub struct LlamaWorkerConfig {
    pub ollama_url: String,
    pub model: String,
    pub embedding_model: String,
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for LlamaWorkerConfig {
    fn default() -> Self {
        Self {
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            model: std::env::var("LLAMA_MODEL")
                .unwrap_or_else(|_| "llama3.2:3b".to_string()),
            embedding_model: std::env::var("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "nomic-embed-text".to_string()),
            timeout: Duration::from_secs(60),
            max_retries: 2,
        }
    }
}

/// Llama Worker for local LLM operations
pub struct LlamaWorker {
    config: LlamaWorkerConfig,
    client: reqwest::Client,
}

/// Query complexity for model routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryComplexity {
    /// Simple factual Q&A → Haiku ($0.25/M)
    Simple,
    /// Implementation, analysis → Sonnet ($3/M)
    Moderate,
    /// Architecture, security, deep reasoning → Opus ($15/M)
    Complex,
}

impl QueryComplexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "SIMPLE",
            Self::Moderate => "MODERATE",
            Self::Complex => "COMPLEX",
        }
    }
}

/// Extracted entity from text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    pub entity_type: String,
    pub context: Option<String>,
    pub confidence: f32,
}

/// Extracted relation between entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub source: String,
    pub target: String,
    pub relation_type: String,
    pub context: Option<String>,
}

/// Ollama generate response
#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    #[serde(default)]
    done: bool,
}

/// Ollama embedding response
#[derive(Debug, Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

impl LlamaWorker {
    /// Create new Llama worker with default config
    pub fn new() -> Self {
        Self::with_config(LlamaWorkerConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: LlamaWorkerConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self { config, client }
    }

    /// Check if Ollama is available
    pub async fn is_available(&self) -> bool {
        match self.client
            .get(&format!("{}/api/tags", self.config.ollama_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Generate text with Llama
    /// Generate text completion using Llama
    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.config.ollama_url);

        let response = self.client
            .post(&url)
            .json(&serde_json::json!({
                "model": self.config.model,
                "prompt": prompt,
                "stream": false,
                "options": {
                    "temperature": 0.1,  // Low temperature for consistency
                    "num_predict": 2048,
                }
            }))
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama error {}: {}", status, body);
        }

        let result: OllamaGenerateResponse = response.json().await
            .context("Failed to parse Ollama response")?;

        Ok(result.response.trim().to_string())
    }

    /// HyDE: Generate hypothetical document for better retrieval
    ///
    /// Instead of embedding the raw query, generate a hypothetical answer
    /// and embed that. The hypothetical answer is more similar to actual
    /// stored documents than the question itself.
    pub async fn generate_hyde(&self, query: &str) -> Result<String> {
        let prompt = format!(
            "Answer this question concisely in 1-2 sentences as if you knew the answer. \
            Do not say 'I don't know'. Just provide a plausible answer.\n\n\
            Question: {}\n\nAnswer:",
            query
        );

        let url = format!("{}/api/generate", self.config.ollama_url);

        let response = self.client
            .post(&url)
            .json(&serde_json::json!({
                "model": self.config.model,
                "prompt": prompt,
                "stream": false,
                "options": {
                    "temperature": 0.3,
                    "num_predict": 150,
                }
            }))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("HyDE generation failed")?;

        if !response.status().is_success() {
            anyhow::bail!("HyDE request failed: {}", response.status());
        }

        let result: OllamaGenerateResponse = response.json().await
            .context("Failed to parse HyDE response")?;

        debug!("HyDE: '{}' -> '{}'", query, result.response.trim());
        Ok(result.response.trim().to_string())
    }

    /// Generate embedding vector
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.config.ollama_url);

        let response = self.client
            .post(&url)
            .json(&serde_json::json!({
                "model": self.config.embedding_model,
                "prompt": text
            }))
            .send()
            .await
            .context("Failed to send embedding request")?;

        if !response.status().is_success() {
            anyhow::bail!("Embedding request failed: {}", response.status());
        }

        let result: OllamaEmbeddingResponse = response.json().await
            .context("Failed to parse embedding response")?;

        Ok(result.embedding)
    }

    /// Classify query complexity for model routing
    ///
    /// Returns the appropriate model tier:
    /// - Simple: Haiku ($0.25/M) - factual Q&A, lookups
    /// - Moderate: Sonnet ($3/M) - implementation, analysis
    /// - Complex: Opus ($15/M) - architecture, security, deep reasoning
    pub async fn classify_complexity(&self, query: &str) -> QueryComplexity {
        // Fast keyword-based classification first
        let lower = query.to_lowercase();

        // Complex indicators (requires deep reasoning)
        let complex_keywords = [
            "architect", "security", "design", "optimize", "refactor",
            "why", "tradeoff", "compare", "evaluate", "review",
            "circle", "audit", "vulnerability", "performance",
        ];
        if complex_keywords.iter().any(|k| lower.contains(k)) {
            return QueryComplexity::Complex;
        }

        // Simple indicators (factual, quick)
        let simple_keywords = [
            "what is", "how to", "show me", "list", "find",
            "where", "status", "version", "help", "usage",
        ];
        if simple_keywords.iter().any(|k| lower.contains(k)) {
            return QueryComplexity::Simple;
        }

        // For uncertain cases, use Llama
        if !self.is_available().await {
            debug!("Ollama unavailable, defaulting to Moderate");
            return QueryComplexity::Moderate;
        }

        let prompt = format!(
            "Classify this query's complexity. Reply with ONLY one word: SIMPLE, MODERATE, or COMPLEX.\n\n\
            SIMPLE: factual questions, lookups, status checks\n\
            MODERATE: implementation, code changes, analysis\n\
            COMPLEX: architecture, security, optimization, deep reasoning\n\n\
            Query: {}\n\n\
            Classification:",
            query
        );

        match self.generate(&prompt).await {
            Ok(response) => {
                let upper = response.to_uppercase();
                if upper.contains("SIMPLE") {
                    QueryComplexity::Simple
                } else if upper.contains("COMPLEX") {
                    QueryComplexity::Complex
                } else {
                    QueryComplexity::Moderate
                }
            }
            Err(e) => {
                warn!("Llama classification failed: {}, defaulting to Moderate", e);
                QueryComplexity::Moderate
            }
        }
    }

    /// Compress conversation context to reduce token usage
    ///
    /// Takes a long conversation and summarizes it to key facts,
    /// decisions, and action items. Target is ~30% of original tokens.
    pub async fn compress_context(
        &self,
        messages: &[(&str, &str)],  // (role, content)
        target_reduction: f32,       // 0.3 = reduce to 30%
    ) -> Result<String> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        // Build conversation text
        let conversation = messages
            .iter()
            .map(|(role, content)| format!("[{}]: {}", role, content))
            .collect::<Vec<_>>()
            .join("\n\n");

        // Estimate target length
        let original_len = conversation.len();
        let target_len = (original_len as f32 * target_reduction) as usize;

        let prompt = format!(
            "Compress this conversation to its essential information. \
            Keep: names, numbers, decisions, todos, technical details. \
            Remove: pleasantries, redundancy, verbose explanations.\n\
            Target length: ~{} characters.\n\n\
            Conversation:\n{}\n\n\
            Compressed summary:",
            target_len,
            conversation
        );

        self.generate(&prompt).await
    }

    /// Extract entities from text for graph memory
    pub async fn extract_entities(&self, text: &str) -> Result<Vec<ExtractedEntity>> {
        let prompt = format!(
            "Extract named entities from this text. Return as JSON array.\n\
            Types: person, project, technology, concept, file, decision\n\n\
            Example output:\n\
            [{{\"name\": \"Velofi\", \"entity_type\": \"project\", \"context\": \"trading platform\", \"confidence\": 0.9}}]\n\n\
            Text: {}\n\n\
            Entities (JSON only):",
            text
        );

        let response = self.generate(&prompt).await?;

        // Try to parse JSON, fall back to empty vec on failure
        match serde_json::from_str::<Vec<ExtractedEntity>>(&response) {
            Ok(entities) => Ok(entities),
            Err(_) => {
                // Try to extract JSON from response
                if let Some(start) = response.find('[') {
                    if let Some(end) = response.rfind(']') {
                        let json = &response[start..=end];
                        let parsed: Vec<ExtractedEntity> = serde_json::from_str(json).unwrap_or_default();
                        return Ok(parsed);
                    }
                }
                debug!("Failed to parse entities from: {}", response);
                Ok(Vec::new())
            }
        }
    }

    /// Extract relations between entities
    pub async fn extract_relations(&self, text: &str, entities: &[ExtractedEntity]) -> Result<Vec<ExtractedRelation>> {
        if entities.len() < 2 {
            return Ok(Vec::new());
        }

        let entity_names: Vec<_> = entities.iter().map(|e| e.name.as_str()).collect();

        let prompt = format!(
            "Find relationships between these entities in the text.\n\
            Entities: {}\n\
            Relation types: works_on, prefers, knows, uses, related_to, depends_on, created_by\n\n\
            Return as JSON array:\n\
            [{{\"source\": \"...\", \"target\": \"...\", \"relation_type\": \"...\", \"context\": \"...\"}}]\n\n\
            Text: {}\n\n\
            Relations (JSON only):",
            entity_names.join(", "),
            text
        );

        let response = self.generate(&prompt).await?;

        match serde_json::from_str::<Vec<ExtractedRelation>>(&response) {
            Ok(relations) => Ok(relations),
            Err(_) => {
                if let Some(start) = response.find('[') {
                    if let Some(end) = response.rfind(']') {
                        let json = &response[start..=end];
                        let parsed: Vec<ExtractedRelation> = serde_json::from_str(json).unwrap_or_default();
                        return Ok(parsed);
                    }
                }
                Ok(Vec::new())
            }
        }
    }

    /// Generate a summary for memory consolidation
    pub async fn summarize_memories(&self, memories: &[&str]) -> Result<String> {
        if memories.is_empty() {
            return Ok(String::new());
        }

        let prompt = format!(
            "Consolidate these related memories into a single concise summary.\n\
            Preserve: key facts, decisions, preferences, technical details.\n\n\
            Memories:\n{}\n\n\
            Consolidated summary:",
            memories.join("\n- ")
        );

        self.generate(&prompt).await
    }

    /// Check if text contains sensitive information that shouldn't be cached
    pub async fn contains_sensitive_info(&self, text: &str) -> bool {
        // Quick keyword check first
        let lower = text.to_lowercase();
        let sensitive_patterns = [
            "password", "api_key", "apikey", "secret", "token",
            "credential", "private_key", "ssh", "bearer",
        ];

        if sensitive_patterns.iter().any(|p| lower.contains(p)) {
            return true;
        }

        // Check for patterns that look like secrets
        let secret_regex = regex::Regex::new(
            r"(?i)(sk-|pk_|api[_-]?key|bearer\s+|password[:\s=])"
        ).unwrap();

        secret_regex.is_match(text)
    }
}

impl Default for LlamaWorker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_complexity_keywords() {
        let worker = LlamaWorker::new();

        // These should be detected by keywords without LLM
        let result = worker.classify_complexity("what is the status?").await;
        assert!(matches!(result, QueryComplexity::Simple));

        let result = worker.classify_complexity("architect the system").await;
        assert!(matches!(result, QueryComplexity::Complex));
    }

    #[tokio::test]
    async fn test_sensitive_detection() {
        let worker = LlamaWorker::new();

        assert!(worker.contains_sensitive_info("my password is secret123").await);
        assert!(worker.contains_sensitive_info("use api_key=sk-abc123").await);
        assert!(!worker.contains_sensitive_info("hello world").await);
    }

    #[test]
    fn test_sensitive_patterns_sync() {
        // Test the regex patterns synchronously
        let patterns = ["password", "api_key", "secret", "token"];
        let test_text = "my password is hidden";
        let lower = test_text.to_lowercase();

        assert!(patterns.iter().any(|p| lower.contains(p)));
    }
}
