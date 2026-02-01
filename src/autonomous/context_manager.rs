//! Context Manager - Proactive Memory Integration
//!
//! Automatically enriches prompts with relevant context from:
//! - Memory store (semantic facts)
//! - Knowledge graph (entities and relations)
//! - Conversation history (recent messages)
//! - Active goals (ongoing tasks)
//!
//! Industry standard: Retrieval-Augmented Generation (RAG) with multi-source fusion

use std::collections::HashSet;
use tracing::{debug, warn};

use crate::conversation::ConversationStore;
use crate::graph::GraphStore;
use crate::llama_worker::LlamaWorker;
use crate::memory::{MemoryStore, ScoredMemory};

use super::goals::{Goal, GoalTracker};

/// Configuration for context enrichment
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Maximum memories to include
    pub max_memories: usize,
    /// Maximum conversation turns to include
    pub max_conversation_turns: usize,
    /// Maximum entities from graph to include
    pub max_entities: usize,
    /// Include active goals in context
    pub include_goals: bool,
    /// Use HyDE for query enhancement
    pub use_hyde: bool,
    /// Minimum relevance score to include memory (0.0-1.0)
    pub min_relevance: f64,
    /// Include user identity context
    pub include_identity: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_memories: 5,
            max_conversation_turns: 5,
            max_entities: 3,
            include_goals: true,
            use_hyde: true,
            min_relevance: 0.1,
            include_identity: true,
        }
    }
}

/// Enriched context ready for prompt injection
#[derive(Debug, Clone)]
pub struct EnrichedContext {
    /// Relevant memories with scores
    pub memories: Vec<ScoredMemory>,
    /// Recent conversation history
    pub conversation: Vec<(String, String)>, // (role, content)
    /// Relevant entities from graph
    pub entities: Vec<GraphEntity>,
    /// Active goals
    pub goals: Vec<Goal>,
    /// User identity context (if available)
    pub identity: Option<String>,
    /// Whether HyDE was used
    pub hyde_used: bool,
    /// Total tokens estimated for context
    pub estimated_tokens: usize,
}

/// Entity from knowledge graph
#[derive(Debug, Clone)]
pub struct GraphEntity {
    pub name: String,
    pub entity_type: String,
    pub relations: Vec<String>,
}

impl EnrichedContext {
    /// Format context for prompt injection
    pub fn format_for_prompt(&self) -> String {
        let mut parts = Vec::new();

        // Recent conversation history (CRITICAL for context continuity)
        if !self.conversation.is_empty() {
            let conv_text = self
                .conversation
                .iter()
                .map(|(role, content)| {
                    let prefix = if role == "user" { "User" } else { "Assistant" };
                    format!("{}: {}", prefix, content)
                })
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("[Recent Conversation]\n{}", conv_text));
        }

        // Identity context
        if let Some(ref identity) = self.identity {
            parts.push(format!("[User Identity]\n{}", identity));
        }

        // Active goals
        if !self.goals.is_empty() {
            let goals_text = self
                .goals
                .iter()
                .map(|g| format!("- {} ({})", g.description, g.status.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("[Active Goals]\n{}", goals_text));
        }

        // Relevant memories
        if !self.memories.is_empty() {
            let memories_text = self
                .memories
                .iter()
                .map(|m| format!("- {}", m.entry.content))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("[Relevant Context]\n{}", memories_text));
        }

        // Related entities
        if !self.entities.is_empty() {
            let entities_text = self
                .entities
                .iter()
                .map(|e| {
                    if e.relations.is_empty() {
                        format!("- {} ({})", e.name, e.entity_type)
                    } else {
                        format!("- {} ({}): {}", e.name, e.entity_type, e.relations.join(", "))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("[Related Entities]\n{}", entities_text));
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("\n\n{}\n", parts.join("\n\n"))
        }
    }

    /// Check if context is empty
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
            && self.conversation.is_empty()
            && self.entities.is_empty()
            && self.goals.is_empty()
            && self.identity.is_none()
    }
}

/// Context manager for proactive memory integration
pub struct ContextManager {
    config: ContextConfig,
}

impl ContextManager {
    /// Create a new context manager
    pub fn new() -> Self {
        Self::with_config(ContextConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: ContextConfig) -> Self {
        Self { config }
    }

    /// Build enriched context for a user prompt
    ///
    /// This is the main entry point - call before sending to Claude API.
    pub async fn build_context(
        &self,
        prompt: &str,
        user_id: i64,
        chat_id: i64,
        memory: &std::sync::Mutex<MemoryStore>,
        conversation: &std::sync::Mutex<ConversationStore>,
        graph: &std::sync::Mutex<GraphStore>,
        goals: Option<&GoalTracker>,
        llama: &LlamaWorker,
    ) -> EnrichedContext {
        let mut context = EnrichedContext {
            memories: vec![],
            conversation: vec![],
            entities: vec![],
            goals: vec![],
            identity: None,
            hyde_used: false,
            estimated_tokens: 0,
        };

        // 1. Get user identity context (highest priority)
        if self.config.include_identity {
            context.identity = self.get_identity_context(user_id, memory);
        }

        // 2. Get recent conversation history FIRST (needed for memory search)
        context.conversation = self.get_conversation_history(chat_id, conversation);

        // 3. Build expanded search query from prompt + recent conversation
        let search_query = self.build_search_query(prompt, &context.conversation);

        // 4. Retrieve relevant memories using expanded query
        context.memories = self.retrieve_memories(&search_query, memory, llama).await;
        context.hyde_used = self.config.use_hyde && llama.is_available().await;

        // 4. Find related entities from graph
        context.entities = self.find_related_entities(prompt, &context.memories, graph);

        // 5. Include active goals if configured
        if self.config.include_goals {
            if let Some(tracker) = goals {
                context.goals = tracker.get_active_goals(user_id).await;
            }
        }

        // Estimate token count
        context.estimated_tokens = self.estimate_tokens(&context);

        debug!(
            "Built context: {} memories, {} entities, {} goals, ~{} tokens",
            context.memories.len(),
            context.entities.len(),
            context.goals.len(),
            context.estimated_tokens
        );

        context
    }

    /// Baseline identity context - always included when no dynamic identity is found
    const BASELINE_IDENTITY: &'static str =
        "I am ClaudeBot, an AI assistant powered by Claude, running as a Telegram bot with persistent memory. \
        I can remember facts from our conversations, learn from interactions, and recall relevant context. \
        I have access to the Claude CLI for coding tasks and can execute commands autonomously.";

    /// Get identity context for user
    fn get_identity_context(&self, _user_id: i64, memory: &std::sync::Mutex<MemoryStore>) -> Option<String> {
        let store = match memory.lock() {
            Ok(s) => s,
            Err(_) => return Some(Self::BASELINE_IDENTITY.to_string()),
        };

        // First, try to find explicit identity memories by category
        if let Ok(results) = store.get_by_category("identity", 3) {
            for result in &results {
                let content = result.content.to_lowercase();
                if content.contains("i am ") || content.contains("my name is") {
                    return Some(result.content.clone());
                }
            }
        }

        // Fallback: search by keywords
        if let Ok(results) = store.search("identity user name role", 5) {
            for result in &results {
                let content = result.entry.content.to_lowercase();
                if content.contains("i am ") || content.contains("my name is") || content.contains("identify as") {
                    return Some(result.entry.content.clone());
                }
            }
        }

        // Always return at least baseline identity
        Some(Self::BASELINE_IDENTITY.to_string())
    }

    /// Retrieve relevant memories using hybrid search with optional HyDE
    async fn retrieve_memories(
        &self,
        prompt: &str,
        memory: &std::sync::Mutex<MemoryStore>,
        llama: &LlamaWorker,
    ) -> Vec<ScoredMemory> {
        // Get embedder for vector search
        let embedder = {
            let store = memory.lock().ok();
            store.and_then(|s| s.get_embedder())
        };

        // Optionally use HyDE for question-like prompts
        let search_text = if self.config.use_hyde && self.is_question(prompt) {
            match llama.generate_hyde(prompt).await {
                Ok(hyde) => hyde,
                Err(_) => prompt.to_string(),
            }
        } else {
            prompt.to_string()
        };

        // Compute embedding outside the lock
        let query_embedding = if let Some(ref embedder) = embedder {
            embedder.read().await.embed(&search_text).await.ok()
        } else {
            None
        };

        // Perform hybrid search
        let store = match memory.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to lock memory store: {}", e);
                return vec![];
            }
        };

        match store.search_hybrid_sync(prompt, query_embedding, self.config.max_memories, 0.4) {
            Ok(results) => {
                debug!(
                    "Memory search returned {} results (min_relevance: {})",
                    results.len(),
                    self.config.min_relevance
                );
                if !results.is_empty() {
                    debug!(
                        "Top result: score={:.3}, content='{}'",
                        results[0].score,
                        &results[0].entry.content[..results[0].entry.content.len().min(50)]
                    );
                }
                results
                    .into_iter()
                    .filter(|r| r.score >= self.config.min_relevance)
                    .collect()
            }
            Err(e) => {
                warn!("Memory hybrid search failed: {}", e);
                vec![]
            }
        }
    }

    /// Build expanded search query from prompt + recent conversation
    /// Short prompts like "yes", "ok", "do it" need context from previous messages
    fn build_search_query(&self, prompt: &str, conversation: &[(String, String)]) -> String {
        let prompt_lower = prompt.to_lowercase().trim().to_string();
        let words: Vec<&str> = prompt.split_whitespace().collect();

        // Detect confirmation/continuation patterns
        let is_confirmation = matches!(
            prompt_lower.as_str(),
            "yes" | "ok" | "sure" | "do it" | "go ahead" | "proceed" |
            "yep" | "yeah" | "correct" | "right" | "continue" | "y" |
            "okay" | "yes please" | "go" | "run it" | "execute"
        );

        if is_confirmation && !conversation.is_empty() {
            // For confirmations, use the last assistant message (what they're confirming)
            if let Some((_, last_assistant)) = conversation.iter()
                .rev()
                .find(|(role, _)| role == "assistant")
            {
                // Extract first 300 chars of assistant's last message
                let context: String = last_assistant.chars().take(300).collect();
                return context;
            }
        }

        // For short non-confirmation queries, combine with recent context
        if words.len() <= 3 && !conversation.is_empty() {
            let recent: Vec<&str> = conversation
                .iter()
                .rev()
                .take(3)
                .map(|(_, content)| content.as_str())
                .collect();

            let combined = format!("{} {}", recent.join(" "), prompt);
            return combined.chars().take(500).collect();
        }

        prompt.to_string()
    }

    /// Get recent conversation history
    fn get_conversation_history(
        &self,
        chat_id: i64,
        conversation: &std::sync::Mutex<ConversationStore>,
    ) -> Vec<(String, String)> {
        let store = match conversation.lock() {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        match store.get_history(chat_id, self.config.max_conversation_turns) {
            Ok(messages) => messages
                .into_iter()
                .map(|m| (m.role, m.content))
                .collect(),
            Err(_) => vec![],
        }
    }

    /// Find related entities from knowledge graph
    fn find_related_entities(
        &self,
        prompt: &str,
        memories: &[ScoredMemory],
        graph: &std::sync::Mutex<GraphStore>,
    ) -> Vec<GraphEntity> {
        let store = match graph.lock() {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        // Extract potential entity names from prompt and memories
        let mut search_terms: HashSet<String> = HashSet::new();

        // Add words from prompt that might be entity names (capitalized words)
        for word in prompt.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
            if clean.len() > 2 && clean.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                search_terms.insert(clean.to_string());
            }
        }

        // Add terms from high-scoring memories
        for memory in memories.iter().take(3) {
            for word in memory.entry.content.split_whitespace() {
                let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
                if clean.len() > 2 && clean.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    search_terms.insert(clean.to_string());
                }
            }
        }

        let mut entities = Vec::new();

        for term in search_terms.iter().take(10) {
            if let Ok(Some(entity)) = store.find_entity_by_name(term) {
                // Get relations for this entity
                let relations = store
                    .get_relations_for_entity(&entity.id)
                    .ok()
                    .unwrap_or_default()
                    .into_iter()
                    .take(3)
                    .map(|r| r.relation_type)
                    .collect();

                entities.push(GraphEntity {
                    name: entity.name,
                    entity_type: entity.entity_type,
                    relations,
                });

                if entities.len() >= self.config.max_entities {
                    break;
                }
            }
        }

        entities
    }

    /// Check if prompt is a question
    fn is_question(&self, prompt: &str) -> bool {
        prompt.contains('?')
            || prompt
                .to_lowercase()
                .split_whitespace()
                .next()
                .map(|w| {
                    ["what", "who", "how", "why", "when", "where", "which", "is", "are", "do", "does", "can", "could", "would", "should"]
                        .contains(&w)
                })
                .unwrap_or(false)
    }

    /// Estimate token count for context
    fn estimate_tokens(&self, context: &EnrichedContext) -> usize {
        let mut tokens = 0;

        // Rough estimation: ~4 characters per token
        if let Some(ref identity) = context.identity {
            tokens += identity.len() / 4;
        }

        for memory in &context.memories {
            tokens += memory.entry.content.len() / 4;
        }

        for (_, content) in &context.conversation {
            tokens += content.len() / 4;
        }

        for entity in &context.entities {
            tokens += (entity.name.len() + entity.entity_type.len() + 20) / 4;
        }

        for goal in &context.goals {
            tokens += goal.description.len() / 4;
        }

        // Add overhead for formatting
        tokens + 50
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_question() {
        let manager = ContextManager::new();

        assert!(manager.is_question("What is the status?"));
        assert!(manager.is_question("How do I fix this?"));
        assert!(manager.is_question("Is this correct"));
        assert!(!manager.is_question("Fix the bug"));
        assert!(!manager.is_question("Update the code"));
    }

    #[test]
    fn test_context_formatting() {
        let context = EnrichedContext {
            memories: vec![],
            conversation: vec![],
            entities: vec![],
            goals: vec![],
            identity: Some("User is Eliot, a developer".to_string()),
            hyde_used: false,
            estimated_tokens: 100,
        };

        let formatted = context.format_for_prompt();
        assert!(formatted.contains("User Identity"));
        assert!(formatted.contains("Eliot"));
    }
}
