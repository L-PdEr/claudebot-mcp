//! Background Processing Module
//!
//! Runs periodic maintenance tasks during idle periods:
//! - Memory consolidation (merge similar memories)
//! - Embedding backfill (generate missing embeddings)
//! - Stale memory cleanup (remove old, unused memories)
//! - Contradiction detection and resolution
//!
//! Industry standard: Event-driven background processing with graceful degradation

use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::embeddings::EmbeddingStore;
use crate::llama_worker::LlamaWorker;
use crate::memory::MemoryStore;

/// Configuration for background processing
#[derive(Debug, Clone)]
pub struct BackgroundConfig {
    /// Interval between consolidation runs
    pub consolidation_interval: Duration,
    /// Interval between embedding backfill runs
    pub backfill_interval: Duration,
    /// Interval between cleanup runs
    pub cleanup_interval: Duration,
    /// Maximum memories to consolidate per run
    pub consolidation_batch_size: usize,
    /// Maximum embeddings to generate per run
    pub backfill_batch_size: usize,
    /// Age in days before memory is considered stale
    pub stale_age_days: i64,
    /// Minimum access count to keep stale memory
    pub stale_min_access_count: i64,
    /// Similarity threshold for consolidation (0.0-1.0)
    pub consolidation_similarity: f32,
    /// Enable background processing
    pub enabled: bool,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            consolidation_interval: Duration::from_secs(300),  // 5 minutes
            backfill_interval: Duration::from_secs(60),        // 1 minute
            cleanup_interval: Duration::from_secs(3600),       // 1 hour
            consolidation_batch_size: 20,
            backfill_batch_size: 50,
            stale_age_days: 90,
            stale_min_access_count: 2,
            consolidation_similarity: 0.85,
            enabled: true,
        }
    }
}

/// Background task types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackgroundTask {
    Consolidation,
    EmbeddingBackfill,
    StaleCleanup,
    ContradictionCheck,
}

impl BackgroundTask {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackgroundTask::Consolidation => "consolidation",
            BackgroundTask::EmbeddingBackfill => "embedding_backfill",
            BackgroundTask::StaleCleanup => "stale_cleanup",
            BackgroundTask::ContradictionCheck => "contradiction_check",
        }
    }
}

/// Statistics for background processing
#[derive(Debug, Default)]
pub struct BackgroundStats {
    pub consolidations_run: AtomicU64,
    pub memories_consolidated: AtomicU64,
    pub backfills_run: AtomicU64,
    pub embeddings_generated: AtomicU64,
    pub cleanups_run: AtomicU64,
    pub memories_removed: AtomicU64,
    pub contradictions_found: AtomicU64,
}

/// Background processor for maintenance tasks
pub struct BackgroundProcessor {
    config: BackgroundConfig,
    stats: Arc<BackgroundStats>,
    running: AtomicBool,
    /// Last run timestamps for each task
    last_runs: Arc<RwLock<std::collections::HashMap<BackgroundTask, i64>>>,
}

impl BackgroundProcessor {
    /// Create a new background processor
    pub fn new() -> Self {
        Self::with_config(BackgroundConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: BackgroundConfig) -> Self {
        Self {
            config,
            stats: Arc::new(BackgroundStats::default()),
            running: AtomicBool::new(false),
            last_runs: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Check if processor is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Run a single iteration of background tasks
    ///
    /// Call this during idle periods (e.g., from lifecycle manager)
    pub async fn run_once(
        &self,
        memory: &std::sync::Mutex<MemoryStore>,
        llama: &LlamaWorker,
    ) -> Result<Vec<(BackgroundTask, usize)>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }

        self.running.store(true, Ordering::SeqCst);
        let mut results = Vec::new();

        let now = chrono::Utc::now().timestamp();

        // Check which tasks are due
        let mut last_runs = self.last_runs.write().await;

        // Embedding backfill (most frequent)
        let last_backfill = last_runs.get(&BackgroundTask::EmbeddingBackfill).copied().unwrap_or(0);
        if now - last_backfill >= self.config.backfill_interval.as_secs() as i64 {
            let count = self.run_embedding_backfill(memory).await?;
            results.push((BackgroundTask::EmbeddingBackfill, count));
            last_runs.insert(BackgroundTask::EmbeddingBackfill, now);
            self.stats.backfills_run.fetch_add(1, Ordering::Relaxed);
            self.stats.embeddings_generated.fetch_add(count as u64, Ordering::Relaxed);
        }

        // Memory consolidation
        let last_consolidation = last_runs.get(&BackgroundTask::Consolidation).copied().unwrap_or(0);
        if now - last_consolidation >= self.config.consolidation_interval.as_secs() as i64 {
            let count = self.run_consolidation(memory, llama).await?;
            results.push((BackgroundTask::Consolidation, count));
            last_runs.insert(BackgroundTask::Consolidation, now);
            self.stats.consolidations_run.fetch_add(1, Ordering::Relaxed);
            self.stats.memories_consolidated.fetch_add(count as u64, Ordering::Relaxed);
        }

        // Stale cleanup (least frequent)
        let last_cleanup = last_runs.get(&BackgroundTask::StaleCleanup).copied().unwrap_or(0);
        if now - last_cleanup >= self.config.cleanup_interval.as_secs() as i64 {
            let count = self.run_stale_cleanup(memory)?;
            results.push((BackgroundTask::StaleCleanup, count));
            last_runs.insert(BackgroundTask::StaleCleanup, now);
            self.stats.cleanups_run.fetch_add(1, Ordering::Relaxed);
            self.stats.memories_removed.fetch_add(count as u64, Ordering::Relaxed);
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(results)
    }

    /// Start continuous background processing loop
    pub async fn run_continuous(
        self: Arc<Self>,
        memory: Arc<std::sync::Mutex<MemoryStore>>,
        llama: Arc<LlamaWorker>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        if !self.config.enabled {
            info!("Background processor disabled");
            return;
        }

        info!("Starting background processor");
        let mut interval = interval(Duration::from_secs(30)); // Check every 30s

        let mut shutdown = shutdown;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.run_once(&memory, &llama).await {
                        warn!("Background task error: {}", e);
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Background processor shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// Run embedding backfill for memories without embeddings
    async fn run_embedding_backfill(
        &self,
        memory: &std::sync::Mutex<MemoryStore>,
    ) -> Result<usize> {
        // Get embedder and memories needing backfill
        let (embedder, memories) = {
            let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            if !store.has_embeddings() {
                return Ok(0);
            }
            let embedder = match store.get_embedder() {
                Some(e) => e,
                None => return Ok(0),
            };
            let memories = store.get_memories_needing_embeddings(self.config.backfill_batch_size)?;
            (embedder, memories)
        };

        if memories.is_empty() {
            return Ok(0);
        }

        debug!("Backfilling {} embeddings", memories.len());

        // Generate embeddings (async, outside lock)
        let mut embeddings: Vec<(String, Vec<f32>)> = Vec::new();
        for (id, content) in &memories {
            match embedder.read().await.embed(content).await {
                Ok(embedding) => {
                    embeddings.push((id.clone(), embedding));
                }
                Err(e) => {
                    debug!("Failed to embed {}: {}", &id[..8.min(id.len())], e);
                }
            }
        }

        // Store embeddings (quick lock)
        let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut count = 0;
        for (id, embedding) in &embeddings {
            if store.store_embedding(id, embedding).is_ok() {
                count += 1;
            }
        }

        if count > 0 {
            info!("Background backfilled {} embeddings", count);
        }

        Ok(count)
    }

    /// Run memory consolidation to merge similar memories
    async fn run_consolidation(
        &self,
        memory: &std::sync::Mutex<MemoryStore>,
        llama: &LlamaWorker,
    ) -> Result<usize> {
        if !llama.is_available().await {
            return Ok(0);
        }

        // Get candidates for consolidation (recent, similar category)
        let candidates = {
            let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            store.get_recent(self.config.consolidation_batch_size)?
        };

        if candidates.len() < 2 {
            return Ok(0);
        }

        // Group by category
        let mut by_category: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
        for mem in candidates {
            by_category.entry(mem.category.clone()).or_default().push(mem);
        }

        let mut consolidated = 0;

        for (_category, memories) in by_category {
            if memories.len() < 2 {
                continue;
            }

            // Find similar pairs using embeddings
            for i in 0..memories.len() {
                for j in (i + 1)..memories.len() {
                    if let (Some(ref emb_i), Some(ref emb_j)) =
                        (&memories[i].embedding, &memories[j].embedding)
                    {
                        let similarity = EmbeddingStore::cosine_similarity(emb_i, emb_j);
                        if similarity >= self.config.consolidation_similarity {
                            // Consolidate these memories
                            let contents: Vec<&str> = vec![
                                &memories[i].content,
                                &memories[j].content,
                            ];

                            if let Ok(summary) = llama.summarize_memories(&contents).await {
                                // Store consolidated and mark old ones
                                let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

                                // Add consolidated memory
                                let _ = store.learn(
                                    &summary,
                                    &memories[i].category,
                                    "consolidation",
                                    0.9,
                                );

                                // We could delete originals here, but for safety we keep them
                                consolidated += 1;
                            }
                        }
                    }
                }
            }
        }

        if consolidated > 0 {
            info!("Consolidated {} memory pairs", consolidated);
        }

        Ok(consolidated)
    }

    /// Remove stale, unused memories
    fn run_stale_cleanup(&self, memory: &std::sync::Mutex<MemoryStore>) -> Result<usize> {
        let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let cutoff = chrono::Utc::now().timestamp() - (self.config.stale_age_days * 86400);

        // Get memories older than cutoff with low access count
        let stale = store.get_recent(1000)?
            .into_iter()
            .filter(|m| {
                m.created_at < cutoff && m.access_count < self.config.stale_min_access_count
            })
            .collect::<Vec<_>>();

        let mut removed = 0;
        for mem in stale {
            if store.forget(&mem.id).is_ok() {
                removed += 1;
            }
        }

        if removed > 0 {
            info!("Cleaned up {} stale memories", removed);
        }

        Ok(removed)
    }

    /// Get current statistics
    pub fn stats(&self) -> &BackgroundStats {
        &self.stats
    }
}

impl Default for BackgroundProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = BackgroundConfig::default();
        assert!(config.enabled);
        assert_eq!(config.consolidation_batch_size, 20);
        assert_eq!(config.backfill_batch_size, 50);
    }

    #[test]
    fn test_task_names() {
        assert_eq!(BackgroundTask::Consolidation.as_str(), "consolidation");
        assert_eq!(BackgroundTask::EmbeddingBackfill.as_str(), "embedding_backfill");
    }
}
