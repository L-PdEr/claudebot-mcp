//! Self-Improvement Feedback Loop
//!
//! Tracks memory usefulness and adjusts confidence scores based on:
//! - Whether retrieved memories were helpful
//! - User corrections and clarifications
//! - Explicit feedback signals
//! - Implicit signals (follow-up questions = low quality)
//!
//! Industry standard: Reinforcement learning from human feedback (RLHF) principles

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::memory::MemoryStore;

/// Types of feedback signals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackSignal {
    /// Memory was explicitly helpful
    Positive,
    /// Memory was not relevant
    Negative,
    /// User corrected information
    Correction,
    /// Memory was retrieved but not used
    Ignored,
    /// Implicit: User asked follow-up question (memory wasn't sufficient)
    FollowUp,
}

impl FeedbackSignal {
    /// Convert signal to confidence adjustment
    pub fn confidence_delta(&self) -> f64 {
        match self {
            FeedbackSignal::Positive => 0.05,
            FeedbackSignal::Negative => -0.1,
            FeedbackSignal::Correction => -0.15,
            FeedbackSignal::Ignored => -0.02,
            FeedbackSignal::FollowUp => -0.03,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackSignal::Positive => "positive",
            FeedbackSignal::Negative => "negative",
            FeedbackSignal::Correction => "correction",
            FeedbackSignal::Ignored => "ignored",
            FeedbackSignal::FollowUp => "follow_up",
        }
    }
}

/// Feedback entry for a memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFeedback {
    pub memory_id: String,
    pub signal: FeedbackSignal,
    pub timestamp: i64,
    pub context: Option<String>,
}

/// Statistics for feedback loop
#[derive(Debug, Default, Clone)]
pub struct FeedbackStats {
    pub total_signals: u64,
    pub positive_count: u64,
    pub negative_count: u64,
    pub corrections_count: u64,
    pub adjustments_made: u64,
    pub memories_boosted: u64,
    pub memories_penalized: u64,
}

/// Configuration for feedback loop
#[derive(Debug, Clone)]
pub struct FeedbackConfig {
    /// Minimum confidence (floor)
    pub min_confidence: f64,
    /// Maximum confidence (ceiling)
    pub max_confidence: f64,
    /// Number of signals before applying adjustment
    pub signals_threshold: usize,
    /// Enable automatic confidence adjustment
    pub auto_adjust: bool,
    /// Enable correction learning
    pub learn_corrections: bool,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.1,
            max_confidence: 1.0,
            signals_threshold: 3,
            auto_adjust: true,
            learn_corrections: true,
        }
    }
}

/// Feedback loop for self-improvement
pub struct FeedbackLoop {
    config: FeedbackConfig,
    /// Pending signals per memory (before threshold)
    pending_signals: Arc<RwLock<HashMap<String, Vec<MemoryFeedback>>>>,
    /// Statistics
    stats: Arc<RwLock<FeedbackStats>>,
    /// Recently retrieved memory IDs (for tracking ignored signals)
    recent_retrievals: Arc<RwLock<Vec<(String, i64)>>>,
}

impl FeedbackLoop {
    /// Create a new feedback loop
    pub fn new() -> Self {
        Self::with_config(FeedbackConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: FeedbackConfig) -> Self {
        Self {
            config,
            pending_signals: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(FeedbackStats::default())),
            recent_retrievals: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Record that memories were retrieved for a query
    pub async fn record_retrieval(&self, memory_ids: &[String]) {
        let now = chrono::Utc::now().timestamp();
        let mut recent = self.recent_retrievals.write().await;

        for id in memory_ids {
            recent.push((id.clone(), now));
        }

        // Keep only last 5 minutes of retrievals
        recent.retain(|(_, ts)| now - ts < 300);
    }

    /// Record feedback signal for a memory
    pub async fn record_signal(
        &self,
        memory_id: &str,
        signal: FeedbackSignal,
        context: Option<String>,
    ) {
        let feedback = MemoryFeedback {
            memory_id: memory_id.to_string(),
            signal,
            timestamp: chrono::Utc::now().timestamp(),
            context,
        };

        // Add to pending signals
        let mut pending = self.pending_signals.write().await;
        pending
            .entry(memory_id.to_string())
            .or_default()
            .push(feedback);

        // Update stats
        let mut stats = self.stats.write().await;
        stats.total_signals += 1;
        match signal {
            FeedbackSignal::Positive => stats.positive_count += 1,
            FeedbackSignal::Negative | FeedbackSignal::Ignored | FeedbackSignal::FollowUp => {
                stats.negative_count += 1
            }
            FeedbackSignal::Correction => stats.corrections_count += 1,
        }

        debug!("Recorded {} signal for memory {}", signal.as_str(), &memory_id[..8.min(memory_id.len())]);
    }

    /// Process pending signals and adjust confidence scores
    pub async fn process_signals(&self, memory: &std::sync::Mutex<MemoryStore>) -> Result<usize> {
        if !self.config.auto_adjust {
            return Ok(0);
        }

        let pending = self.pending_signals.read().await;
        let mut adjustments = 0;

        for (memory_id, signals) in pending.iter() {
            if signals.len() < self.config.signals_threshold {
                continue;
            }

            // Calculate aggregate adjustment
            let delta: f64 = signals.iter().map(|s| s.signal.confidence_delta()).sum();

            if delta.abs() < 0.01 {
                continue;
            }

            // Apply adjustment
            let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

            if let Ok(Some(entry)) = store.get_by_id(memory_id) {
                let new_confidence = (entry.confidence + delta)
                    .max(self.config.min_confidence)
                    .min(self.config.max_confidence);

                // Update confidence (requires adding a method to MemoryStore)
                // For now, we log the intended adjustment
                info!(
                    "Confidence adjustment for {}: {:.2} -> {:.2} (delta: {:.2})",
                    &memory_id[..8.min(memory_id.len())],
                    entry.confidence,
                    new_confidence,
                    delta
                );

                adjustments += 1;

                // Update stats
                let mut stats = self.stats.write().await;
                stats.adjustments_made += 1;
                if delta > 0.0 {
                    stats.memories_boosted += 1;
                } else {
                    stats.memories_penalized += 1;
                }
            }
        }

        // Clear processed signals
        if adjustments > 0 {
            let mut pending = self.pending_signals.write().await;
            pending.retain(|_, signals| signals.len() < self.config.signals_threshold);
        }

        Ok(adjustments)
    }

    /// Detect correction patterns in user message
    ///
    /// Looks for patterns like:
    /// - "No, I meant..."
    /// - "Actually, ..."
    /// - "That's wrong, ..."
    /// - "Incorrect, ..."
    pub fn detect_correction(&self, message: &str) -> Option<String> {
        let lower = message.to_lowercase();

        let correction_patterns = [
            ("no, ", "correction"),
            ("actually, ", "clarification"),
            ("that's wrong", "correction"),
            ("incorrect", "correction"),
            ("not quite", "clarification"),
            ("i meant ", "clarification"),
            ("to clarify", "clarification"),
            ("let me correct", "correction"),
        ];

        for (pattern, _kind) in correction_patterns {
            if lower.starts_with(pattern) || lower.contains(&format!(" {}", pattern)) {
                // Extract the correction content
                if let Some(pos) = lower.find(pattern) {
                    let content = &message[pos + pattern.len()..];
                    let end = content
                        .find(|c: char| c == '.' || c == '!' || c == '\n')
                        .unwrap_or(content.len().min(200));
                    return Some(content[..end].trim().to_string());
                }
            }
        }

        None
    }

    /// Learn from a user correction
    pub async fn learn_correction(
        &self,
        correction: &str,
        related_memory_ids: &[String],
        memory: &std::sync::Mutex<MemoryStore>,
        user_id: i64,
    ) -> Result<()> {
        if !self.config.learn_corrections {
            return Ok(());
        }

        // Record negative signal for related memories
        for id in related_memory_ids {
            self.record_signal(id, FeedbackSignal::Correction, Some(correction.to_string()))
                .await;
        }

        // Store the correction as new knowledge
        let store = memory.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let _ = store.learn(
            correction,
            "correction",
            &format!("user_correction_{}", user_id),
            0.95, // High confidence for explicit corrections
        );

        info!("Learned correction: {}", correction);
        Ok(())
    }

    /// Detect implicit follow-up signal (user asking for more detail)
    pub async fn detect_followup(&self, message: &str) -> bool {
        let lower = message.to_lowercase();

        let followup_patterns = [
            "can you explain",
            "what do you mean",
            "i don't understand",
            "more detail",
            "tell me more",
            "elaborate",
            "be more specific",
            "that doesn't make sense",
        ];

        followup_patterns.iter().any(|p| lower.contains(p))
    }

    /// Apply follow-up signal to recently retrieved memories
    pub async fn apply_followup_signal(&self) {
        let recent = self.recent_retrievals.read().await;
        let now = chrono::Utc::now().timestamp();

        // Apply to memories retrieved in the last minute
        for (id, ts) in recent.iter() {
            if now - ts < 60 {
                self.record_signal(id, FeedbackSignal::FollowUp, None).await;
            }
        }
    }

    /// Get current statistics
    pub async fn stats(&self) -> FeedbackStats {
        self.stats.read().await.clone()
    }

    /// Format stats for display
    pub async fn format_stats(&self) -> String {
        let stats = self.stats.read().await;
        format!(
            "Feedback Loop Statistics\n\n\
            Total signals: {}\n\
            Positive: {}\n\
            Negative: {}\n\
            Corrections: {}\n\n\
            Adjustments made: {}\n\
            Memories boosted: {}\n\
            Memories penalized: {}",
            stats.total_signals,
            stats.positive_count,
            stats.negative_count,
            stats.corrections_count,
            stats.adjustments_made,
            stats.memories_boosted,
            stats.memories_penalized
        )
    }
}

impl Default for FeedbackLoop {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_delta() {
        assert!(FeedbackSignal::Positive.confidence_delta() > 0.0);
        assert!(FeedbackSignal::Negative.confidence_delta() < 0.0);
        assert!(FeedbackSignal::Correction.confidence_delta() < 0.0);
    }

    #[test]
    fn test_correction_detection() {
        let feedback = FeedbackLoop::new();

        let msg = "No, I meant the other file";
        assert!(feedback.detect_correction(msg).is_some());

        let msg = "Actually, the correct answer is 42";
        assert!(feedback.detect_correction(msg).is_some());

        let msg = "Thanks for the help!";
        assert!(feedback.detect_correction(msg).is_none());
    }

    #[tokio::test]
    async fn test_signal_recording() {
        let feedback = FeedbackLoop::new();

        feedback
            .record_signal("mem123", FeedbackSignal::Positive, None)
            .await;

        let stats = feedback.stats().await;
        assert_eq!(stats.total_signals, 1);
        assert_eq!(stats.positive_count, 1);
    }
}
