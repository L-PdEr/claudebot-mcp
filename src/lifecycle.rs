//! Wake/Sleep Lifecycle Management
//!
//! Implements MemGPT-style wake/sleep cycle for background processing:
//! - **Wake**: Active message processing
//! - **Sleep**: Memory consolidation, decay, compression
//! - **Processing**: Currently handling a request
//!
//! Background tasks run during idle periods to optimize memory and reduce costs.

use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

/// Lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    /// Bot is idle, background tasks running
    Sleep = 0,
    /// Bot is awake, ready for messages
    Wake = 1,
    /// Bot is actively processing a message
    Processing = 2,
}

impl From<u8> for State {
    fn from(v: u8) -> Self {
        match v {
            0 => State::Sleep,
            1 => State::Wake,
            2 => State::Processing,
            _ => State::Wake,
        }
    }
}

/// Configuration for the lifecycle manager
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// How long to wait before transitioning to sleep (default: 5 min)
    pub idle_timeout: Duration,
    /// Interval for background tasks during sleep (default: 60s)
    pub sleep_task_interval: Duration,
    /// Enable memory consolidation during sleep
    pub enable_consolidation: bool,
    /// Enable Ebbinghaus decay during sleep
    pub enable_decay: bool,
    /// Enable context compression during sleep
    pub enable_compression: bool,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(300), // 5 minutes
            sleep_task_interval: Duration::from_secs(60),
            enable_consolidation: true,
            enable_decay: true,
            enable_compression: true,
        }
    }
}

/// Lifecycle manager for wake/sleep cycle
pub struct LifecycleManager {
    state: AtomicU8,
    last_activity: AtomicI64,
    config: LifecycleConfig,
    wake_notify: Notify,
    stats: LifecycleStats,
}

/// Statistics for lifecycle monitoring
#[derive(Debug, Default)]
pub struct LifecycleStats {
    pub wake_count: AtomicI64,
    pub sleep_count: AtomicI64,
    pub consolidations: AtomicI64,
    pub decays_applied: AtomicI64,
    pub compressions: AtomicI64,
}

impl LifecycleManager {
    /// Create a new lifecycle manager
    pub fn new(config: LifecycleConfig) -> Arc<Self> {
        Arc::new(Self {
            state: AtomicU8::new(State::Wake as u8),
            last_activity: AtomicI64::new(chrono::Utc::now().timestamp()),
            config,
            wake_notify: Notify::new(),
            stats: LifecycleStats::default(),
        })
    }

    /// Create with default configuration
    pub fn with_defaults() -> Arc<Self> {
        Self::new(LifecycleConfig::default())
    }

    /// Get current state
    pub fn current_state(&self) -> State {
        State::from(self.state.load(Ordering::Relaxed))
    }

    /// Check if currently sleeping
    pub fn is_sleeping(&self) -> bool {
        self.current_state() == State::Sleep
    }

    /// Check if currently processing
    pub fn is_processing(&self) -> bool {
        self.current_state() == State::Processing
    }

    /// Get seconds since last activity
    pub fn idle_seconds(&self) -> i64 {
        let now = chrono::Utc::now().timestamp();
        let last = self.last_activity.load(Ordering::Relaxed);
        now - last
    }

    /// Record activity (resets idle timer, wakes if sleeping)
    pub fn record_activity(&self) {
        self.last_activity.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);

        let current = self.current_state();
        if current == State::Sleep {
            self.transition_to(State::Wake);
            self.stats.wake_count.fetch_add(1, Ordering::Relaxed);
            self.wake_notify.notify_one();
            debug!("Woke from sleep due to activity");
        }
    }

    /// Mark as processing (prevents sleep during active work)
    pub fn start_processing(&self) {
        self.record_activity();
        self.transition_to(State::Processing);
    }

    /// Mark processing complete
    pub fn end_processing(&self) {
        self.record_activity();
        self.transition_to(State::Wake);
    }

    /// Manually force sleep state (for /sleep command)
    /// Returns true if transitioned, false if already sleeping or processing
    pub fn force_sleep(&self) -> bool {
        let current = self.current_state();
        if current == State::Processing {
            // Don't interrupt active work
            return false;
        }
        if current == State::Sleep {
            // Already sleeping
            return false;
        }
        self.transition_to(State::Sleep);
        self.stats.sleep_count.fetch_add(1, Ordering::Relaxed);
        info!("Forced sleep via command");
        true
    }

    /// Manually force wake state (for /wake command)
    pub fn force_wake(&self) {
        self.record_activity();
        if self.current_state() == State::Sleep {
            self.transition_to(State::Wake);
            self.stats.wake_count.fetch_add(1, Ordering::Relaxed);
            self.wake_notify.notify_one();
            info!("Forced wake via command");
        }
    }

    /// Transition to a new state
    fn transition_to(&self, new_state: State) {
        let old = self.state.swap(new_state as u8, Ordering::Relaxed);
        if old != new_state as u8 {
            debug!("Lifecycle: {:?} -> {:?}", State::from(old), new_state);
        }
    }

    /// Run the lifecycle loop (call this in a background task)
    pub async fn run(self: Arc<Self>, callbacks: LifecycleCallbacks) {
        info!("Lifecycle manager started");

        loop {
            match self.current_state() {
                State::Sleep => {
                    // Run background tasks
                    if self.config.enable_consolidation {
                        if let Some(ref cb) = callbacks.on_consolidate {
                            if let Err(e) = cb().await {
                                warn!("Consolidation failed: {}", e);
                            } else {
                                self.stats.consolidations.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    if self.config.enable_decay {
                        if let Some(ref cb) = callbacks.on_decay {
                            if let Err(e) = cb().await {
                                warn!("Decay failed: {}", e);
                            } else {
                                self.stats.decays_applied.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    if self.config.enable_compression {
                        if let Some(ref cb) = callbacks.on_compress {
                            if let Err(e) = cb().await {
                                warn!("Compression failed: {}", e);
                            } else {
                                self.stats.compressions.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }

                    // Wait for wake signal or interval
                    tokio::select! {
                        _ = self.wake_notify.notified() => {
                            debug!("Lifecycle: woken by notification");
                        }
                        _ = tokio::time::sleep(self.config.sleep_task_interval) => {
                            debug!("Lifecycle: sleep interval elapsed");
                        }
                    }
                }

                State::Wake => {
                    // Check for idle timeout
                    let idle = self.idle_seconds();
                    let timeout_secs = self.config.idle_timeout.as_secs() as i64;

                    if idle > timeout_secs {
                        self.transition_to(State::Sleep);
                        self.stats.sleep_count.fetch_add(1, Ordering::Relaxed);
                        info!("Entering sleep mode after {}s idle", idle);
                    } else {
                        // Short sleep before checking again
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }

                State::Processing => {
                    // Don't interrupt active processing
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Get lifecycle statistics
    pub fn get_stats(&self) -> LifecycleStatsSnapshot {
        LifecycleStatsSnapshot {
            current_state: self.current_state(),
            idle_seconds: self.idle_seconds(),
            wake_count: self.stats.wake_count.load(Ordering::Relaxed),
            sleep_count: self.stats.sleep_count.load(Ordering::Relaxed),
            consolidations: self.stats.consolidations.load(Ordering::Relaxed),
            decays_applied: self.stats.decays_applied.load(Ordering::Relaxed),
            compressions: self.stats.compressions.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of lifecycle statistics
#[derive(Debug, Clone)]
pub struct LifecycleStatsSnapshot {
    pub current_state: State,
    pub idle_seconds: i64,
    pub wake_count: i64,
    pub sleep_count: i64,
    pub consolidations: i64,
    pub decays_applied: i64,
    pub compressions: i64,
}

/// Callbacks for lifecycle events
pub struct LifecycleCallbacks {
    /// Called during sleep to consolidate similar memories
    pub on_consolidate: Option<Box<dyn Fn() -> futures_util::future::BoxFuture<'static, anyhow::Result<()>> + Send + Sync>>,
    /// Called during sleep to apply Ebbinghaus decay
    pub on_decay: Option<Box<dyn Fn() -> futures_util::future::BoxFuture<'static, anyhow::Result<()>> + Send + Sync>>,
    /// Called during sleep to compress old conversations
    pub on_compress: Option<Box<dyn Fn() -> futures_util::future::BoxFuture<'static, anyhow::Result<()>> + Send + Sync>>,
}

impl Default for LifecycleCallbacks {
    fn default() -> Self {
        Self {
            on_consolidate: None,
            on_decay: None,
            on_compress: None,
        }
    }
}

/// Guard that marks processing start/end automatically
pub struct ProcessingGuard {
    lifecycle: Arc<LifecycleManager>,
}

impl ProcessingGuard {
    /// Create a new processing guard
    pub fn new(lifecycle: Arc<LifecycleManager>) -> Self {
        lifecycle.start_processing();
        Self { lifecycle }
    }
}

impl Drop for ProcessingGuard {
    fn drop(&mut self) {
        self.lifecycle.end_processing();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        let manager = LifecycleManager::with_defaults();

        assert_eq!(manager.current_state(), State::Wake);

        manager.start_processing();
        assert_eq!(manager.current_state(), State::Processing);

        manager.end_processing();
        assert_eq!(manager.current_state(), State::Wake);
    }

    #[test]
    fn test_activity_recording() {
        let manager = LifecycleManager::with_defaults();

        // Force to sleep
        manager.transition_to(State::Sleep);
        assert!(manager.is_sleeping());

        // Activity should wake
        manager.record_activity();
        assert!(!manager.is_sleeping());
        assert_eq!(manager.current_state(), State::Wake);
    }

    #[test]
    fn test_idle_seconds() {
        let manager = LifecycleManager::with_defaults();
        manager.record_activity();

        // Should be very small
        assert!(manager.idle_seconds() < 2);
    }

    #[test]
    fn test_processing_guard() {
        let manager = LifecycleManager::with_defaults();

        {
            let _guard = ProcessingGuard::new(Arc::clone(&manager));
            assert!(manager.is_processing());
        }

        // Guard dropped, should be back to Wake
        assert!(!manager.is_processing());
    }
}
