//! Hybrid Memory Store
//!
//! Vector search + BM25 keyword matching for memory retrieval.
//! Uses SQLite for persistence with optional Ollama embeddings.

use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::embeddings::{embedding_from_bytes, embedding_to_bytes, EmbeddingConfig, EmbeddingStore};

/// Memory entry
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub category: String,
    pub source: String,
    pub confidence: f64,
    pub created_at: i64,
    pub access_count: i64,
    pub embedding: Option<Vec<f32>>,
}

/// Search result with score breakdown
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub entry: MemoryEntry,
    pub score: f64,
    pub keyword_score: f64,
    pub vector_score: f64,
}

/// Legacy search result (for backwards compatibility)
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: MemoryEntry,
    pub score: f64,
}

impl From<ScoredMemory> for SearchResult {
    fn from(sm: ScoredMemory) -> Self {
        SearchResult {
            entry: sm.entry,
            score: sm.score,
        }
    }
}

/// Memory store with SQLite backend and optional embeddings
pub struct MemoryStore {
    conn: Connection,
    embedder: Option<Arc<RwLock<EmbeddingStore>>>,
}

impl MemoryStore {
    /// Open or create memory database (sync, no embeddings)
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let store = Self {
            conn,
            embedder: None,
        };
        store.init_schema()?;

        info!("Memory store opened: {}", path.display());
        Ok(store)
    }

    /// Open with embedding support (async)
    pub async fn open_with_embeddings(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;

        // Try to initialize embedder
        let embedder = {
            let store = EmbeddingStore::new(EmbeddingConfig::default());
            if store.check_availability().await {
                info!("Embedding service available - semantic search enabled");
                Some(Arc::new(RwLock::new(store)))
            } else {
                warn!("Embedding service unavailable - using keyword search only");
                None
            }
        };

        let store = Self { conn, embedder };
        store.init_schema()?;

        info!("Memory store opened with embeddings: {}", path.display());
        Ok(store)
    }

    /// Set embedder (for testing or late initialization)
    pub fn set_embedder(&mut self, embedder: EmbeddingStore) {
        self.embedder = Some(Arc::new(RwLock::new(embedder)));
    }

    /// Check if embeddings are available
    pub fn has_embeddings(&self) -> bool {
        self.embedder.is_some()
    }

    /// Get a clone of the embedder Arc for async operations outside the mutex
    pub fn get_embedder(&self) -> Option<Arc<RwLock<EmbeddingStore>>> {
        self.embedder.clone()
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        // Base schema (without embedding column for backwards compatibility)
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'general',
                source TEXT NOT NULL DEFAULT 'user',
                confidence REAL NOT NULL DEFAULT 0.8,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                last_accessed INTEGER,
                access_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid'
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            "#,
        )?;

        // Migration: Add embedding column if it doesn't exist
        let _ = self.conn.execute(
            "ALTER TABLE memories ADD COLUMN embedding BLOB",
            [],
        );

        // Create embedding index (after migration ensures column exists)
        let _ = self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_memories_has_embedding ON memories(embedding IS NOT NULL)",
            [],
        );

        Ok(())
    }

    /// Generate content hash for ID
    fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(&hasher.finalize()[..16])
    }

    /// Store a memory (sync, no embedding)
    pub fn learn(&self, content: &str, category: &str, source: &str, confidence: f64) -> Result<String> {
        let id = Self::hash_content(content);

        self.conn.execute(
            r#"
            INSERT INTO memories (id, content, category, source, confidence)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                confidence = MAX(confidence, excluded.confidence),
                access_count = access_count + 1,
                last_accessed = unixepoch()
            "#,
            params![id, content, category, source, confidence],
        )?;

        debug!("Learned: {} ({})", &id[..8], category);
        Ok(id)
    }

    /// Store a memory with embedding (async)
    pub async fn learn_with_embedding(
        &self,
        content: &str,
        category: &str,
        source: &str,
        confidence: f64,
    ) -> Result<String> {
        let id = Self::hash_content(content);

        // Generate embedding if available
        let embedding_bytes = if let Some(ref embedder) = self.embedder {
            match embedder.read().await.embed(content).await {
                Ok(emb) => Some(embedding_to_bytes(&emb)),
                Err(e) => {
                    warn!("Failed to generate embedding: {}", e);
                    None
                }
            }
        } else {
            None
        };

        self.conn.execute(
            r#"
            INSERT INTO memories (id, content, category, source, confidence, embedding)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(id) DO UPDATE SET
                confidence = MAX(confidence, excluded.confidence),
                access_count = access_count + 1,
                last_accessed = unixepoch(),
                embedding = COALESCE(excluded.embedding, embedding)
            "#,
            params![id, content, category, source, confidence, embedding_bytes],
        )?;

        debug!("Learned with embedding: {} ({})", &id[..8], category);
        Ok(id)
    }

    /// Search memories using FTS (keyword search)
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT m.id, m.content, m.category, m.source, m.confidence,
                   m.created_at, m.access_count, m.embedding,
                   bm25(memories_fts) as score
            FROM memories_fts
            JOIN memories m ON memories_fts.rowid = m.rowid
            WHERE memories_fts MATCH ?1
            ORDER BY score
            LIMIT ?2
            "#,
        )?;

        // FTS5 query: wrap terms in quotes for phrase matching
        let fts_query = query
            .split_whitespace()
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");

        if fts_query.is_empty() {
            return Ok(vec![]);
        }

        let results = stmt
            .query_map(params![fts_query, limit], |row| {
                let embedding_bytes: Option<Vec<u8>> = row.get(7)?;
                Ok(SearchResult {
                    entry: MemoryEntry {
                        id: row.get(0)?,
                        content: row.get(1)?,
                        category: row.get(2)?,
                        source: row.get(3)?,
                        confidence: row.get(4)?,
                        created_at: row.get(5)?,
                        access_count: row.get(6)?,
                        embedding: embedding_bytes.map(|b| embedding_from_bytes(&b)),
                    },
                    score: row.get::<_, f64>(8)?.abs(), // BM25 returns negative
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Hybrid search combining BM25 keywords + vector similarity
    ///
    /// # Arguments
    /// * `query` - Search query
    /// * `limit` - Maximum results
    /// * `keyword_weight` - Weight for keyword score (0.0-1.0, default 0.4)
    pub async fn search_hybrid(
        &self,
        query: &str,
        limit: usize,
        keyword_weight: f32,
    ) -> Result<Vec<ScoredMemory>> {
        // 1. Get keyword results (BM25)
        let keyword_results = self.search(query, limit * 3)?;

        // 2. Get vector results if embedder available
        let query_embedding = if let Some(ref embedder) = self.embedder {
            embedder.read().await.embed(query).await.ok()
        } else {
            None
        };

        let vector_results = if let Some(ref query_vec) = query_embedding {
            self.search_by_embedding(query_vec, limit * 3)?
        } else {
            vec![]
        };

        // 3. Fuse results
        let fused = self.fuse_results(keyword_results, vector_results, keyword_weight);

        // 4. Return top-k
        Ok(fused.into_iter().take(limit).collect())
    }

    /// Hybrid search with pre-computed embedding (sync version)
    ///
    /// Use this when you've already computed the query embedding outside the mutex lock.
    pub fn search_hybrid_sync(
        &self,
        query: &str,
        query_embedding: Option<Vec<f32>>,
        limit: usize,
        keyword_weight: f32,
    ) -> Result<Vec<ScoredMemory>> {
        // 1. Get keyword results (BM25)
        let keyword_results = self.search(query, limit * 3)?;

        // 2. Get vector results if embedding provided
        let vector_results = if let Some(ref query_vec) = query_embedding {
            self.search_by_embedding(query_vec, limit * 3)?
        } else {
            vec![]
        };

        // 3. Fuse results
        let fused = self.fuse_results(keyword_results, vector_results, keyword_weight);

        // 4. Return top-k
        Ok(fused.into_iter().take(limit).collect())
    }

    /// Search by embedding similarity only
    fn search_by_embedding(&self, query_vec: &[f32], limit: usize) -> Result<Vec<(String, f64)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, embedding
            FROM memories
            WHERE embedding IS NOT NULL
            "#,
        )?;

        let mut results: Vec<(String, f64)> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let embedding_bytes: Vec<u8> = row.get(1)?;
                let embedding = embedding_from_bytes(&embedding_bytes);
                let score = EmbeddingStore::cosine_similarity(query_vec, &embedding) as f64;
                Ok((id, score))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Fuse keyword and vector results with score normalization
    fn fuse_results(
        &self,
        keyword_results: Vec<SearchResult>,
        vector_results: Vec<(String, f64)>,
        keyword_weight: f32,
    ) -> Vec<ScoredMemory> {
        let vector_weight = 1.0 - keyword_weight;

        // Build ID -> keyword score map (normalize BM25 scores)
        let max_keyword = keyword_results
            .iter()
            .map(|r| r.score)
            .fold(0.0f64, |a, b| a.max(b));

        let keyword_map: HashMap<String, (MemoryEntry, f64)> = keyword_results
            .into_iter()
            .map(|r| {
                let norm_score = if max_keyword > 0.0 {
                    r.score / max_keyword
                } else {
                    0.0
                };
                (r.entry.id.clone(), (r.entry, norm_score))
            })
            .collect();

        // Build ID -> vector score map (already 0-1 from cosine)
        let vector_map: HashMap<String, f64> = vector_results.into_iter().collect();

        // Combine all unique IDs
        let mut all_ids: Vec<String> = keyword_map.keys().cloned().collect();
        for id in vector_map.keys() {
            if !keyword_map.contains_key(id) {
                all_ids.push(id.clone());
            }
        }

        // Calculate hybrid scores
        let mut results: Vec<ScoredMemory> = all_ids
            .into_iter()
            .filter_map(|id| {
                let (entry, kw_score) = if let Some((e, s)) = keyword_map.get(&id) {
                    (e.clone(), *s)
                } else {
                    // Load entry from DB if only in vector results
                    match self.get_by_id(&id) {
                        Ok(Some(e)) => (e, 0.0),
                        _ => return None,
                    }
                };

                let vec_score = *vector_map.get(&id).unwrap_or(&0.0);

                let hybrid_score =
                    (kw_score * keyword_weight as f64) + (vec_score * vector_weight as f64);

                Some(ScoredMemory {
                    entry,
                    score: hybrid_score,
                    keyword_score: kw_score,
                    vector_score: vec_score,
                })
            })
            .collect();

        // Sort by hybrid score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        results
    }

    /// Get memory by ID
    pub fn get_by_id(&self, id: &str) -> Result<Option<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, content, category, source, confidence, created_at, access_count, embedding
            FROM memories
            WHERE id = ?1
            "#,
        )?;

        let result = stmt.query_row(params![id], |row| {
            let embedding_bytes: Option<Vec<u8>> = row.get(7)?;
            Ok(MemoryEntry {
                id: row.get(0)?,
                content: row.get(1)?,
                category: row.get(2)?,
                source: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get(5)?,
                access_count: row.get(6)?,
                embedding: embedding_bytes.map(|b| embedding_from_bytes(&b)),
            })
        });

        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get memories that need embeddings (sync)
    pub fn get_memories_needing_embeddings(&self, batch_size: usize) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, content
            FROM memories
            WHERE embedding IS NULL
            LIMIT ?1
            "#,
        )?;

        let memories: Vec<(String, String)> = stmt
            .query_map(params![batch_size], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(memories)
    }

    /// Store a single embedding (sync)
    pub fn store_embedding(&self, id: &str, embedding: &[f32]) -> Result<()> {
        let bytes = embedding_to_bytes(embedding);
        self.conn.execute(
            "UPDATE memories SET embedding = ?1 WHERE id = ?2",
            params![bytes, id],
        )?;
        Ok(())
    }

    /// Backfill embeddings for memories that don't have them
    pub async fn backfill_embeddings(&self, batch_size: usize) -> Result<usize> {
        let embedder = match &self.embedder {
            Some(e) => e,
            None => {
                warn!("No embedder available for backfill");
                return Ok(0);
            }
        };

        // Get memories without embeddings
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, content
            FROM memories
            WHERE embedding IS NULL
            LIMIT ?1
            "#,
        )?;

        let memories: Vec<(String, String)> = stmt
            .query_map(params![batch_size], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let total = memories.len();
        let mut embedded = 0;

        for (id, content) in memories {
            match embedder.read().await.embed(&content).await {
                Ok(embedding) => {
                    let bytes = embedding_to_bytes(&embedding);
                    self.conn.execute(
                        "UPDATE memories SET embedding = ?1 WHERE id = ?2",
                        params![bytes, id],
                    )?;
                    embedded += 1;
                    debug!("Backfilled embedding for {}", &id[..8]);
                }
                Err(e) => {
                    warn!("Failed to backfill embedding for {}: {}", &id[..8], e);
                }
            }
        }

        info!("Backfilled {}/{} memories with embeddings", embedded, total);
        Ok(embedded)
    }

    /// Get embedding statistics
    pub fn embedding_stats(&self) -> Result<EmbeddingStats> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;

        let with_embedding: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE embedding IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        Ok(EmbeddingStats {
            total_memories: total as usize,
            with_embeddings: with_embedding as usize,
            without_embeddings: (total - with_embedding) as usize,
            coverage_percent: if total > 0 {
                (with_embedding as f64 / total as f64) * 100.0
            } else {
                0.0
            },
        })
    }

    /// Get memories by category
    pub fn get_by_category(&self, category: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, content, category, source, confidence, created_at, access_count, embedding
            FROM memories
            WHERE category = ?1
            ORDER BY created_at DESC
            LIMIT ?2
            "#,
        )?;

        let results = stmt
            .query_map(params![category, limit], |row| {
                let embedding_bytes: Option<Vec<u8>> = row.get(7)?;
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    category: row.get(2)?,
                    source: row.get(3)?,
                    confidence: row.get(4)?,
                    created_at: row.get(5)?,
                    access_count: row.get(6)?,
                    embedding: embedding_bytes.map(|b| embedding_from_bytes(&b)),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Get recent memories
    pub fn get_recent(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, content, category, source, confidence, created_at, access_count, embedding
            FROM memories
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
        )?;

        let results = stmt
            .query_map(params![limit], |row| {
                let embedding_bytes: Option<Vec<u8>> = row.get(7)?;
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    category: row.get(2)?,
                    source: row.get(3)?,
                    confidence: row.get(4)?,
                    created_at: row.get(5)?,
                    access_count: row.get(6)?,
                    embedding: embedding_bytes.map(|b| embedding_from_bytes(&b)),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Delete a memory
    pub fn forget(&self, id: &str) -> Result<bool> {
        let rows = self.conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Get memory stats
    pub fn stats(&self) -> Result<MemoryStats> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;

        let mut stmt = self
            .conn
            .prepare("SELECT category, COUNT(*) FROM memories GROUP BY category")?;
        let by_category: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(MemoryStats {
            total_entries: total as usize,
            by_category,
        })
    }
}

/// Memory statistics
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub total_entries: usize,
    pub by_category: Vec<(String, i64)>,
}

/// Embedding statistics
#[derive(Debug, Clone, Default)]
pub struct EmbeddingStats {
    pub total_memories: usize,
    pub with_embeddings: usize,
    pub without_embeddings: usize,
    pub coverage_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db(name: &str) -> MemoryStore {
        let path = PathBuf::from(format!("/tmp/claudebot_test_{}.db", name));
        let _ = std::fs::remove_file(&path);
        MemoryStore::open(&path).unwrap()
    }

    #[test]
    fn test_learn_and_search() {
        let store = temp_db("search");

        store
            .learn("Rust is a systems programming language", "tech", "test", 0.9)
            .unwrap();
        store
            .learn("Vue is a JavaScript framework", "tech", "test", 0.8)
            .unwrap();

        let results = store.search("Rust programming", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].entry.content.contains("Rust"));
    }

    #[test]
    fn test_categories() {
        let store = temp_db("categories");

        store.learn("Fact 1", "facts", "test", 0.9).unwrap();
        store.learn("Fact 2", "facts", "test", 0.9).unwrap();
        store.learn("Preference 1", "preferences", "test", 0.9).unwrap();

        let facts = store.get_by_category("facts", 10).unwrap();
        assert_eq!(facts.len(), 2);

        let stats = store.stats().unwrap();
        assert_eq!(stats.total_entries, 3);
    }

    #[test]
    fn test_embedding_stats() {
        let store = temp_db("embedding_stats");

        store.learn("Memory 1", "test", "test", 0.9).unwrap();
        store.learn("Memory 2", "test", "test", 0.9).unwrap();

        let stats = store.embedding_stats().unwrap();
        assert_eq!(stats.total_memories, 2);
        assert_eq!(stats.with_embeddings, 0);
        assert_eq!(stats.without_embeddings, 2);
    }

    #[test]
    fn test_get_by_id() {
        let store = temp_db("get_by_id");

        let id = store.learn("Test memory", "test", "test", 0.9).unwrap();
        let entry = store.get_by_id(&id).unwrap();

        assert!(entry.is_some());
        assert_eq!(entry.unwrap().content, "Test memory");

        let missing = store.get_by_id("nonexistent").unwrap();
        assert!(missing.is_none());
    }
}
