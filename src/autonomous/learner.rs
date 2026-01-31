//! Autonomous Learning Module
//!
//! Automatically extracts and stores knowledge from conversations without
//! explicit user commands. Implements industry-standard patterns:
//!
//! - Fact extraction using LLM
//! - Entity recognition and graph building
//! - Preference detection from behavior
//! - Confidence scoring based on evidence

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::graph::GraphStore;
use crate::llama_worker::LlamaWorker;
use crate::memory::MemoryStore;

/// Configuration for autonomous learning
#[derive(Debug, Clone)]
pub struct LearningConfig {
    /// Minimum confidence to auto-store a fact (0.0-1.0)
    pub min_confidence: f32,
    /// Enable automatic fact extraction
    pub auto_extract_facts: bool,
    /// Enable automatic entity extraction
    pub auto_extract_entities: bool,
    /// Enable preference learning from behavior
    pub learn_preferences: bool,
    /// Minimum message length to analyze
    pub min_message_length: usize,
    /// Patterns to skip (greetings, confirmations)
    pub skip_patterns: Vec<String>,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.7,
            auto_extract_facts: true,
            auto_extract_entities: true,
            learn_preferences: true,
            min_message_length: 20,
            skip_patterns: vec![
                "hi".to_string(),
                "hello".to_string(),
                "thanks".to_string(),
                "ok".to_string(),
                "yes".to_string(),
                "no".to_string(),
            ],
        }
    }
}

/// A fact learned from conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedFact {
    pub content: String,
    pub category: String,
    pub confidence: f32,
    pub source_message: String,
    pub entities: Vec<String>,
}

/// Statistics about learning activity
#[derive(Debug, Default, Clone)]
pub struct LearningStats {
    pub messages_analyzed: u64,
    pub facts_extracted: u64,
    pub entities_found: u64,
    pub preferences_learned: u64,
    pub duplicates_skipped: u64,
}

/// Autonomous learner that extracts knowledge from conversations
pub struct AutonomousLearner {
    config: LearningConfig,
    stats: Arc<RwLock<LearningStats>>,
    /// Cache of recently learned content hashes to avoid duplicates
    recent_hashes: Arc<RwLock<HashMap<String, i64>>>,
}

impl AutonomousLearner {
    /// Create a new autonomous learner with default config
    pub fn new() -> Self {
        Self::with_config(LearningConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: LearningConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(LearningStats::default())),
            recent_hashes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Analyze a user message and extract learnable content
    ///
    /// This is the main entry point - call on every user message.
    /// Returns extracted facts ready to be stored.
    pub async fn analyze_message(
        &self,
        message: &str,
        user_id: i64,
        llama: &LlamaWorker,
    ) -> Vec<LearnedFact> {
        let mut stats = self.stats.write().await;
        stats.messages_analyzed += 1;
        drop(stats);

        // Skip short messages
        if message.len() < self.config.min_message_length {
            debug!("Message too short to analyze: {} chars", message.len());
            return vec![];
        }

        // Skip common patterns
        let lower = message.to_lowercase();
        for pattern in &self.config.skip_patterns {
            if lower.trim() == pattern.as_str() || lower.starts_with(&format!("{} ", pattern)) {
                debug!("Skipping message matching pattern: {}", pattern);
                return vec![];
            }
        }

        // Check if we've recently processed similar content
        let content_hash = self.hash_content(message);
        {
            let recent = self.recent_hashes.read().await;
            if recent.contains_key(&content_hash) {
                let mut stats = self.stats.write().await;
                stats.duplicates_skipped += 1;
                return vec![];
            }
        }

        let mut facts = Vec::new();

        // 1. Extract explicit facts (statements of information)
        if self.config.auto_extract_facts {
            if let Ok(extracted) = self.extract_facts(message, llama).await {
                for fact in extracted {
                    if fact.confidence >= self.config.min_confidence {
                        facts.push(fact);
                    }
                }
            }
        }

        // 2. Detect preferences from language patterns
        if self.config.learn_preferences {
            if let Some(pref) = self.detect_preference_sync(message, user_id) {
                facts.push(pref);
            }
        }

        // Mark content as processed
        if !facts.is_empty() {
            let mut recent = self.recent_hashes.write().await;
            let now = chrono::Utc::now().timestamp();
            recent.insert(content_hash, now);

            // Clean old entries (older than 1 hour)
            recent.retain(|_, &mut ts| now - ts < 3600);

            let mut stats = self.stats.write().await;
            stats.facts_extracted += facts.len() as u64;
        }

        facts
    }

    /// Extract entities from a message and store in graph
    pub async fn extract_and_store_entities(
        &self,
        message: &str,
        llama: &LlamaWorker,
        graph: &std::sync::Mutex<GraphStore>,
    ) -> Result<usize> {
        if !self.config.auto_extract_entities {
            return Ok(0);
        }

        // Use Llama to extract entities
        let entities = llama.extract_entities(message).await?;

        if entities.is_empty() {
            return Ok(0);
        }

        // Also extract relations between entities
        let relations = llama.extract_relations(message, &entities).await?;

        // Store in graph
        let store = graph.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stored = 0;

        for entity in &entities {
            let attrs = entity.context.as_ref().map(|ctx| {
                serde_json::json!({
                    "context": ctx,
                    "confidence": entity.confidence,
                    "auto_extracted": true
                })
            });

            if store.add_entity(&entity.entity_type, &entity.name, attrs).is_ok() {
                stored += 1;
            }
        }

        // Store relations
        for relation in relations {
            // Find entity IDs
            if let (Some(source), Some(target)) = (
                store.find_entity_by_name(&relation.source).ok().flatten(),
                store.find_entity_by_name(&relation.target).ok().flatten(),
            ) {
                let _ = store.add_relation(&source.id, &target.id, &relation.relation_type, Some(0.8));
            }
        }

        let mut stats = self.stats.write().await;
        stats.entities_found += stored as u64;

        info!("Auto-extracted {} entities from message", stored);
        Ok(stored)
    }

    /// Store learned facts in memory
    pub async fn store_facts(
        &self,
        facts: &[LearnedFact],
        user_id: i64,
        memory: &std::sync::Mutex<MemoryStore>,
    ) -> Result<usize> {
        if facts.is_empty() {
            return Ok(0);
        }

        let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let source = format!("auto_learn_user_{}", user_id);
        let mut stored = 0;

        for fact in facts {
            match store.learn(&fact.content, &fact.category, &source, fact.confidence as f64) {
                Ok(id) => {
                    debug!("Auto-stored fact: {} ({})", &id[..8], fact.category);
                    stored += 1;
                }
                Err(e) => {
                    warn!("Failed to store fact: {}", e);
                }
            }
        }

        Ok(stored)
    }

    /// Extract facts using LLM
    async fn extract_facts(&self, message: &str, llama: &LlamaWorker) -> Result<Vec<LearnedFact>> {
        // Only process if Ollama is available
        if !llama.is_available().await {
            return Ok(vec![]);
        }

        let prompt = format!(
            r#"Extract factual information from this message that would be useful to remember.
Return as JSON array. Only include clear facts, not opinions or questions.

Example output:
[{{"content": "User prefers Rust over Python", "category": "preference", "confidence": 0.9}}]

Categories: preference, project, technical, personal, task, decision

Message: {}

Facts (JSON only, empty array if no facts):"#,
            message
        );

        let response = llama.generate(&prompt).await?;

        // Parse JSON response
        let facts: Vec<LearnedFact> = self.parse_facts_response(&response, message);
        Ok(facts)
    }

    /// Parse LLM response into facts
    fn parse_facts_response(&self, response: &str, source_message: &str) -> Vec<LearnedFact> {
        // Try to find JSON array in response
        let json_str = if let Some(start) = response.find('[') {
            if let Some(end) = response.rfind(']') {
                &response[start..=end]
            } else {
                return vec![];
            }
        } else {
            return vec![];
        };

        #[derive(Deserialize)]
        struct RawFact {
            content: String,
            category: String,
            confidence: f32,
        }

        match serde_json::from_str::<Vec<RawFact>>(json_str) {
            Ok(raw_facts) => raw_facts
                .into_iter()
                .map(|f| LearnedFact {
                    content: f.content,
                    category: f.category,
                    confidence: f.confidence,
                    source_message: source_message.to_string(),
                    entities: vec![],
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    /// Detect preferences from language patterns (sync, no LLM)
    /// This is a fast pattern-based detector that doesn't require Ollama.
    pub fn detect_preference_sync(&self, message: &str, _user_id: i64) -> Option<LearnedFact> {
        let lower = message.to_lowercase();

        // Common preference patterns
        let preference_patterns = [
            ("i prefer ", "preference"),
            ("i like ", "preference"),
            ("i want ", "preference"),
            ("i need ", "requirement"),
            ("i always ", "habit"),
            ("i never ", "habit"),
            ("i use ", "tool"),
            ("my favorite ", "preference"),
        ];

        for (pattern, category) in preference_patterns {
            if let Some(pos) = lower.find(pattern) {
                let content = &message[pos..];
                // Extract until end of sentence
                let end = content
                    .find(|c: char| c == '.' || c == '!' || c == '?' || c == '\n')
                    .unwrap_or(content.len().min(100));

                let fact_content = content[..end].to_string();

                if fact_content.len() > 10 {
                    return Some(LearnedFact {
                        content: fact_content,
                        category: category.to_string(),
                        confidence: 0.85,
                        source_message: message.to_string(),
                        entities: vec![],
                    });
                }
            }
        }

        None
    }

    /// Simple content hash for deduplication
    fn hash_content(&self, content: &str) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(content.to_lowercase().as_bytes());
        hex::encode(&hasher.finalize()[..8])
    }

    /// Get current learning statistics
    pub async fn stats(&self) -> LearningStats {
        self.stats.read().await.clone()
    }
}

impl Default for AutonomousLearner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preference_detection() {
        let learner = AutonomousLearner::new();

        let msg = "I prefer using Rust for systems programming";
        let pref = learner.detect_preference_sync(msg, 123);

        assert!(pref.is_some());
        let fact = pref.unwrap();
        assert_eq!(fact.category, "preference");
        assert!(fact.content.contains("prefer"));
    }

    #[test]
    fn test_skip_patterns() {
        let learner = AutonomousLearner::new();

        // Short messages should be skipped
        let config = &learner.config;
        assert!(config.skip_patterns.contains(&"hi".to_string()));
        assert!(config.skip_patterns.contains(&"thanks".to_string()));
    }
}
