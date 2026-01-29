//! Vector Embeddings for Semantic Search
//!
//! Provides semantic similarity search using local embeddings via Ollama.
//! Falls back to keyword-based FTS5 search if Ollama is unavailable.
//!
//! Supports hybrid retrieval: combines FTS5 keyword scores with vector similarity.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, warn};

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
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            model: std::env::var("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "nomic-embed-text".to_string()),
            dimension: 768, // nomic-embed-text default
            timeout: Duration::from_secs(30),
        }
    }
}

/// Embedding generator and similarity search
pub struct EmbeddingStore {
    config: EmbeddingConfig,
    client: reqwest::Client,
    available: std::sync::atomic::AtomicBool,
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

        Self {
            config,
            client,
            available: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(EmbeddingConfig::default())
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

    /// Generate embedding for text
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
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
