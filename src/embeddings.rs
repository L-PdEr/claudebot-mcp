//! Vector Embeddings for Semantic Search
//!
//! Provides semantic similarity search using local embeddings via Ollama.
//! Falls back to keyword-based FTS5 search if Ollama is unavailable.
//!
//! Supports hybrid retrieval: combines FTS5 keyword scores with vector similarity.
//! Includes LRU caching for query embeddings to reduce latency.

use anyhow::{Context, Result};
use moka::future::Cache;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{info, warn};

/// Embedding store configuration
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Ollama API URL
    pub ollama_url: String,
    /// Embedding model name
    pub model: String,
    /// Embedding dimension (depends on model)
    pub dimension: usize,
    /// Request timeout
    pub timeout: Duration,
    /// Reranker model (optional, for cross-encoder reranking)
    pub reranker_model: Option<String>,
}

/// Get embedding dimension for known models
fn model_dimension(model: &str) -> usize {
    match model {
        "mxbai-embed-large" => 1024,
        "snowflake-arctic-embed" | "snowflake-arctic-embed-m" => 768,
        "nomic-embed-text" => 768,
        "all-minilm" | "all-minilm-l6-v2" => 384,
        "bge-large" | "bge-large-en" => 1024,
        "bge-base" | "bge-base-en" => 768,
        _ => 768, // Default fallback
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        let model = std::env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text".to_string()); // Must match llama_worker.rs (768 dim)
        let dimension = model_dimension(&model);

        Self {
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            model,
            dimension,
            timeout: Duration::from_secs(30),
            reranker_model: std::env::var("RERANKER_MODEL").ok(), // e.g., "bge-reranker-base"
        }
    }
}

/// Embedding generator and similarity search
pub struct EmbeddingStore {
    config: EmbeddingConfig,
    client: reqwest::Client,
    available: std::sync::atomic::AtomicBool,
    /// LRU cache for query embeddings (max 1000 entries, 1 hour TTL)
    cache: Cache<String, Vec<f32>>,
    /// Cache statistics
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

/// Ollama embedding response
#[derive(Debug, Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

impl EmbeddingStore {
    /// Create a new embedding store
    pub fn new(config: EmbeddingConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to create HTTP client");

        // LRU cache: 1000 entries, 1 hour TTL
        let cache = Cache::builder()
            .max_capacity(1000)
            .time_to_live(Duration::from_secs(3600))
            .build();

        Self {
            config,
            client,
            available: std::sync::atomic::AtomicBool::new(true),
            cache,
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(EmbeddingConfig::default())
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (u64, u64) {
        (
            self.cache_hits.load(Ordering::Relaxed),
            self.cache_misses.load(Ordering::Relaxed),
        )
    }

    /// Check if Ollama is available
    pub async fn check_availability(&self) -> bool {
        match self.client
            .get(&format!("{}/api/tags", self.config.ollama_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) => {
                let available = resp.status().is_success();
                self.available.store(available, std::sync::atomic::Ordering::Relaxed);
                available
            }
            Err(_) => {
                self.available.store(false, std::sync::atomic::Ordering::Relaxed);
                false
            }
        }
    }

    /// Check cached availability (fast, non-blocking)
    pub fn is_available(&self) -> bool {
        self.available.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Generate embedding for text (with caching)
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if !self.is_available() {
            anyhow::bail!("Embedding service unavailable");
        }

        // Normalize text for cache key (trim, lowercase for queries)
        let cache_key = text.trim().to_string();

        // Check cache first
        if let Some(cached) = self.cache.get(&cache_key).await {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(cached);
        }
        self.cache_misses.fetch_add(1, Ordering::Relaxed);

        // Not in cache, compute embedding
        let embedding = self.embed_uncached(text).await?;

        // Store in cache
        self.cache.insert(cache_key, embedding.clone()).await;

        Ok(embedding)
    }

    /// Generate embedding without caching (for storage, not queries)
    pub async fn embed_uncached(&self, text: &str) -> Result<Vec<f32>> {
        if !self.is_available() {
            anyhow::bail!("Embedding service unavailable");
        }

        let url = format!("{}/api/embeddings", self.config.ollama_url);

        let response = self.client
            .post(&url)
            .json(&serde_json::json!({
                "model": self.config.model,
                "prompt": text
            }))
            .send()
            .await
            .context("Failed to send embedding request")?;

        if !response.status().is_success() {
            let status = response.status();
            self.available.store(false, std::sync::atomic::Ordering::Relaxed);
            anyhow::bail!("Embedding request failed: {}", status);
        }

        let result: OllamaEmbeddingResponse = response.json().await
            .context("Failed to parse embedding response")?;

        Ok(result.embedding)
    }

    /// Generate embeddings for multiple texts (batched)
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut embeddings = Vec::with_capacity(texts.len());

        for text in texts {
            match self.embed(text).await {
                Ok(emb) => embeddings.push(emb),
                Err(e) => {
                    warn!("Failed to embed text: {}", e);
                    // Return zero vector as fallback
                    embeddings.push(vec![0.0; self.config.dimension]);
                }
            }
        }

        Ok(embeddings)
    }

    /// Calculate cosine similarity between two vectors
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot / (norm_a * norm_b)
    }

    /// Check if reranker is configured
    pub fn has_reranker(&self) -> bool {
        self.config.reranker_model.is_some()
    }

    /// Rerank documents using cross-encoder model
    ///
    /// Takes a query and list of (id, content) pairs, returns reranked (id, score) pairs.
    /// Uses LLM to score relevance of each document to the query.
    pub async fn rerank(
        &self,
        query: &str,
        documents: Vec<(String, String)>,
        top_k: usize,
    ) -> Result<Vec<(String, f64)>> {
        let reranker = match &self.config.reranker_model {
            Some(model) => model,
            None => return Ok(documents.into_iter().map(|(id, _)| (id, 1.0)).collect()),
        };

        let url = format!("{}/api/generate", self.config.ollama_url);
        let mut results: Vec<(String, f64)> = Vec::with_capacity(documents.len());

        for (id, content) in documents {
            // Truncate long documents for reranking
            let doc_preview = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            // Cross-encoder reranking prompt
            let prompt = format!(
                "Rate the relevance of this document to the query on a scale of 0.0 to 1.0.\n\n\
                Query: {}\n\n\
                Document: {}\n\n\
                Return only a decimal number between 0.0 and 1.0:",
                query, doc_preview
            );

            match self.client
                .post(&url)
                .json(&serde_json::json!({
                    "model": reranker,
                    "prompt": prompt,
                    "stream": false,
                    "options": {
                        "temperature": 0.0,
                        "num_predict": 10
                    }
                }))
                .timeout(Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        if let Some(response) = json.get("response").and_then(|r| r.as_str()) {
                            // Parse score from response
                            let score = response
                                .trim()
                                .parse::<f64>()
                                .unwrap_or(0.5)
                                .clamp(0.0, 1.0);
                            results.push((id, score));
                            continue;
                        }
                    }
                }
                _ => {}
            }
            // Fallback: keep original with neutral score
            results.push((id, 0.5));
        }

        // Sort by score descending and take top_k
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);

        Ok(results)
    }

    /// Find most similar vectors from a collection
    pub fn find_similar(
        query: &[f32],
        candidates: &[(String, Vec<f32>)],
        top_k: usize,
    ) -> Vec<(String, f32)> {
        let mut scores: Vec<(String, f32)> = candidates
            .iter()
            .map(|(id, emb)| (id.clone(), Self::cosine_similarity(query, emb)))
            .collect();

        // Sort by similarity (descending)
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scores.truncate(top_k);
        scores
    }

    /// Hybrid score combining keyword and vector similarity
    ///
    /// # Arguments
    /// * `keyword_score` - BM25 or FTS5 score (normalized 0-1)
    /// * `vector_score` - Cosine similarity (0-1)
    /// * `keyword_weight` - Weight for keyword score (default: 0.4)
    pub fn hybrid_score(keyword_score: f32, vector_score: f32, keyword_weight: f32) -> f32 {
        let vector_weight = 1.0 - keyword_weight;
        keyword_score * keyword_weight + vector_score * vector_weight
    }

    /// Normalize a score to 0-1 range
    pub fn normalize_score(score: f32, max_score: f32) -> f32 {
        if max_score <= 0.0 {
            0.0
        } else {
            (score / max_score).min(1.0)
        }
    }
}

/// Embedding with metadata
#[derive(Debug, Clone)]
pub struct EmbeddedEntry {
    pub id: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: serde_json::Value,
}

impl EmbeddedEntry {
    pub fn new(id: String, content: String, embedding: Vec<f32>) -> Self {
        Self {
            id,
            content,
            embedding,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// In-memory vector index for fast similarity search
pub struct VectorIndex {
    entries: Vec<EmbeddedEntry>,
    dimension: usize,
}

impl VectorIndex {
    /// Create a new vector index
    pub fn new(dimension: usize) -> Self {
        Self {
            entries: Vec::new(),
            dimension,
        }
    }

    /// Add an entry to the index
    pub fn add(&mut self, entry: EmbeddedEntry) {
        if entry.embedding.len() == self.dimension {
            self.entries.push(entry);
        } else {
            warn!(
                "Embedding dimension mismatch: expected {}, got {}",
                self.dimension,
                entry.embedding.len()
            );
        }
    }

    /// Search for similar entries
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<(&EmbeddedEntry, f32)> {
        let mut results: Vec<(&EmbeddedEntry, f32)> = self.entries
            .iter()
            .map(|e| (e, EmbeddingStore::cosine_similarity(query_embedding, &e.embedding)))
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        results
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if index is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Serialize embedding to bytes for SQLite BLOB storage
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter()
        .flat_map(|f| f.to_le_bytes())
        .collect()
}

/// Deserialize embedding from bytes
pub fn embedding_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((EmbeddingStore::cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!(EmbeddingStore::cosine_similarity(&a, &c).abs() < 0.001);

        let d = vec![-1.0, 0.0, 0.0];
        assert!((EmbeddingStore::cosine_similarity(&a, &d) + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_hybrid_score() {
        let score = EmbeddingStore::hybrid_score(0.8, 0.6, 0.4);
        // 0.8 * 0.4 + 0.6 * 0.6 = 0.32 + 0.36 = 0.68
        assert!((score - 0.68).abs() < 0.001);
    }

    #[test]
    fn test_find_similar() {
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![
            ("a".to_string(), vec![1.0, 0.0, 0.0]),
            ("b".to_string(), vec![0.0, 1.0, 0.0]),
            ("c".to_string(), vec![0.7, 0.7, 0.0]),
        ];

        let results = EmbeddingStore::find_similar(&query, &candidates, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert!((results[0].1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_vector_index() {
        let mut index = VectorIndex::new(3);

        index.add(EmbeddedEntry::new("1".to_string(), "test".to_string(), vec![1.0, 0.0, 0.0]));
        index.add(EmbeddedEntry::new("2".to_string(), "test2".to_string(), vec![0.0, 1.0, 0.0]));

        assert_eq!(index.len(), 2);

        let results = index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, "1");
    }

    #[test]
    fn test_embedding_serialization() {
        let embedding = vec![1.0, 2.5, -3.0, 0.0];
        let bytes = embedding_to_bytes(&embedding);
        let restored = embedding_from_bytes(&bytes);

        assert_eq!(embedding.len(), restored.len());
        for (a, b) in embedding.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 0.0001);
        }
    }
}
