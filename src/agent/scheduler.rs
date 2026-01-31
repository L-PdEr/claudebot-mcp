//! Proactive Notification Scheduler
//!
//! Implements scheduled tasks and reminders:
//! - Cron-like scheduling
//! - One-time and recurring reminders
//! - Priority-based notification queue
//! - User preference-aware delivery
//!
//! Industry standard: Temporal workflows, Celery beat

use chrono::Timelike;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

/// Type of notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationType {
    /// User-set reminder
    Reminder,
    /// Goal progress update
    GoalUpdate,
    /// Learning insight
    LearningInsight,
    /// System status
    SystemStatus,
    /// Scheduled task result
    TaskResult,
    /// Proactive suggestion
    Suggestion,
}

impl NotificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reminder => "reminder",
            Self::GoalUpdate => "goal_update",
            Self::LearningInsight => "learning",
            Self::SystemStatus => "system",
            Self::TaskResult => "task",
            Self::Suggestion => "suggestion",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Reminder => "⏰",
            Self::GoalUpdate => "🎯",
            Self::LearningInsight => "💡",
            Self::SystemStatus => "ℹ️",
            Self::TaskResult => "✅",
            Self::Suggestion => "💬",
        }
    }
}

/// Priority level for notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Low = 1,
    Normal = 2,
    High = 3,
    Urgent = 4,
}

/// A reminder to be delivered
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    /// Unique ID
    pub id: String,
    /// User to notify
    pub user_id: i64,
    /// Chat to send to
    pub chat_id: i64,
    /// Reminder message
    pub message: String,
    /// When to deliver (unix timestamp)
    pub due_at: i64,
    /// Type of notification
    pub notification_type: NotificationType,
    /// Priority
    pub priority: Priority,
    /// Whether it recurs
    pub recurring: Option<RecurrenceRule>,
    /// Creation timestamp
    pub created_at: i64,
}

impl Reminder {
    /// Create a one-time reminder
    pub fn once(user_id: i64, chat_id: i64, message: &str, due_at: i64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            chat_id,
            message: message.to_string(),
            due_at,
            notification_type: NotificationType::Reminder,
            priority: Priority::Normal,
            recurring: None,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    /// Set priority
    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Set notification type
    pub fn with_type(mut self, notification_type: NotificationType) -> Self {
        self.notification_type = notification_type;
        self
    }

    /// Make recurring
    pub fn recurring(mut self, rule: RecurrenceRule) -> Self {
        self.recurring = Some(rule);
        self
    }

    /// Check if due
    pub fn is_due(&self) -> bool {
        chrono::Utc::now().timestamp() >= self.due_at
    }

    /// Get next occurrence (for recurring)
    pub fn next_occurrence(&self) -> Option<i64> {
        self.recurring.as_ref().map(|rule| rule.next_from(self.due_at))
    }

    /// Format for display
    pub fn format(&self) -> String {
        let due = chrono::DateTime::from_timestamp(self.due_at, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        format!(
            "{} {} (due: {})",
            self.notification_type.emoji(),
            self.message,
            due
        )
    }
}

/// Recurrence rule for repeating reminders
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurrenceRule {
    /// Interval type
    pub interval: RecurrenceInterval,
    /// How many intervals between occurrences
    pub every: u32,
    /// Maximum occurrences (None = infinite)
    pub max_occurrences: Option<u32>,
    /// Occurrences so far
    pub occurrences: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RecurrenceInterval {
    Minutes,
    Hours,
    Days,
    Weeks,
}

impl RecurrenceRule {
    /// Create daily recurrence
    pub fn daily() -> Self {
        Self {
            interval: RecurrenceInterval::Days,
            every: 1,
            max_occurrences: None,
            occurrences: 0,
        }
    }

    /// Create weekly recurrence
    pub fn weekly() -> Self {
        Self {
            interval: RecurrenceInterval::Weeks,
            every: 1,
            max_occurrences: None,
            occurrences: 0,
        }
    }

    /// Create hourly recurrence
    pub fn hourly() -> Self {
        Self {
            interval: RecurrenceInterval::Hours,
            every: 1,
            max_occurrences: None,
            occurrences: 0,
        }
    }

    /// Set interval count
    pub fn every(mut self, count: u32) -> Self {
        self.every = count.max(1);
        self
    }

    /// Limit occurrences
    pub fn times(mut self, count: u32) -> Self {
        self.max_occurrences = Some(count);
        self
    }

    /// Calculate next occurrence from a given time
    pub fn next_from(&self, from: i64) -> i64 {
        let seconds = match self.interval {
            RecurrenceInterval::Minutes => 60 * self.every as i64,
            RecurrenceInterval::Hours => 3600 * self.every as i64,
            RecurrenceInterval::Days => 86400 * self.every as i64,
            RecurrenceInterval::Weeks => 604800 * self.every as i64,
        };
        from + seconds
    }

    /// Check if more occurrences allowed
    pub fn has_more(&self) -> bool {
        self.max_occurrences.map(|m| self.occurrences < m).unwrap_or(true)
    }
}

/// A scheduled task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    /// Unique ID
    pub id: String,
    /// Task name
    pub name: String,
    /// Task description/payload
    pub payload: String,
    /// When to execute
    pub execute_at: i64,
    /// Priority
    pub priority: Priority,
    /// Callback identifier
    pub callback: String,
    /// Whether task is active
    pub active: bool,
    /// Recurrence rule
    pub recurring: Option<RecurrenceRule>,
}

impl ScheduledTask {
    /// Create a new scheduled task
    pub fn new(name: &str, payload: &str, execute_at: i64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            payload: payload.to_string(),
            execute_at,
            priority: Priority::Normal,
            callback: String::new(),
            active: true,
            recurring: None,
        }
    }

    /// Set callback identifier
    pub fn with_callback(mut self, callback: &str) -> Self {
        self.callback = callback.to_string();
        self
    }

    /// Check if due
    pub fn is_due(&self) -> bool {
        self.active && chrono::Utc::now().timestamp() >= self.execute_at
    }
}

/// Entry in the priority queue
#[derive(Debug, Clone)]
struct QueueEntry {
    due_at: i64,
    priority: Priority,
    id: String,
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for QueueEntry {}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap, so higher values get popped first
        // We want: higher priority first, then earlier due time first
        match self.priority.cmp(&other.priority) {
            Ordering::Equal => other.due_at.cmp(&self.due_at), // Earlier (smaller) due_at should be "greater"
            ord => ord, // Higher priority should be "greater"
        }
    }
}

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// How often to check for due items
    pub poll_interval: Duration,
    /// Maximum concurrent notifications
    pub max_concurrent: usize,
    /// Quiet hours start (0-23)
    pub quiet_start: u8,
    /// Quiet hours end (0-23)
    pub quiet_end: u8,
    /// Enable quiet hours
    pub enable_quiet_hours: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(10),
            max_concurrent: 10,
            quiet_start: 22,
            quiet_end: 8,
            enable_quiet_hours: false,
        }
    }
}

/// The scheduler for managing reminders and tasks
pub struct Scheduler {
    config: SchedulerConfig,
    reminders: Arc<RwLock<HashMap<String, Reminder>>>,
    tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
    queue: Arc<RwLock<BinaryHeap<QueueEntry>>>,
    notification_tx: mpsc::Sender<Reminder>,
    running: Arc<RwLock<bool>>,
}

impl Scheduler {
    /// Create a new scheduler
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<Reminder>) {
        Self::with_config(SchedulerConfig::default(), buffer_size)
    }

    /// Create with custom config
    pub fn with_config(config: SchedulerConfig, buffer_size: usize) -> (Self, mpsc::Receiver<Reminder>) {
        let (tx, rx) = mpsc::channel(buffer_size);

        let scheduler = Self {
            config,
            reminders: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            queue: Arc::new(RwLock::new(BinaryHeap::new())),
            notification_tx: tx,
            running: Arc::new(RwLock::new(false)),
        };

        (scheduler, rx)
    }

    /// Schedule a reminder
    pub async fn schedule_reminder(&self, reminder: Reminder) -> String {
        let id = reminder.id.clone();

        // Add to queue
        {
            let mut queue = self.queue.write().await;
            queue.push(QueueEntry {
                due_at: reminder.due_at,
                priority: reminder.priority,
                id: id.clone(),
            });
        }

        // Store reminder
        self.reminders.write().await.insert(id.clone(), reminder);

        info!("Scheduled reminder: {}", id);
        id
    }

    /// Schedule a task
    pub async fn schedule_task(&self, task: ScheduledTask) -> String {
        let id = task.id.clone();
        self.tasks.write().await.insert(id.clone(), task);
        info!("Scheduled task: {}", id);
        id
    }

    /// Cancel a reminder
    pub async fn cancel_reminder(&self, id: &str) -> bool {
        self.reminders.write().await.remove(id).is_some()
    }

    /// Cancel a task
    pub async fn cancel_task(&self, id: &str) -> bool {
        if let Some(task) = self.tasks.write().await.get_mut(id) {
            task.active = false;
            true
        } else {
            false
        }
    }

    /// Get reminders for a user
    pub async fn get_user_reminders(&self, user_id: i64) -> Vec<Reminder> {
        self.reminders
            .read()
            .await
            .values()
            .filter(|r| r.user_id == user_id)
            .cloned()
            .collect()
    }

    /// Check if in quiet hours
    fn is_quiet_hour(&self) -> bool {
        if !self.config.enable_quiet_hours {
            return false;
        }

        let hour = chrono::Local::now().hour() as u8;
        if self.config.quiet_start < self.config.quiet_end {
            hour >= self.config.quiet_start && hour < self.config.quiet_end
        } else {
            hour >= self.config.quiet_start || hour < self.config.quiet_end
        }
    }

    /// Process due reminders
    async fn process_due(&self) -> usize {
        if self.is_quiet_hour() {
            return 0;
        }

        let mut processed = 0;

        // Collect due reminders
        let due_ids: Vec<String> = {
            let reminders = self.reminders.read().await;
            reminders
                .values()
                .filter(|r| r.is_due())
                .map(|r| r.id.clone())
                .collect()
        };

        for id in due_ids {
            let reminder = {
                let mut reminders = self.reminders.write().await;
                if let Some(reminder) = reminders.remove(&id) {
                    // Handle recurrence
                    if let Some(rule) = &reminder.recurring {
                        if rule.has_more() {
                            let mut next_rule = rule.clone();
                            next_rule.occurrences += 1;
                            let mut next = reminder.clone();
                            next.id = uuid::Uuid::new_v4().to_string();
                            next.due_at = rule.next_from(reminder.due_at);
                            next.recurring = Some(next_rule);
                            reminders.insert(next.id.clone(), next);
                        }
                    }
                    Some(reminder)
                } else {
                    None
                }
            };

            if let Some(reminder) = reminder {
                if self.notification_tx.send(reminder).await.is_ok() {
                    processed += 1;
                }
            }
        }

        processed
    }

    /// Start the scheduler loop
    pub async fn start(&self) {
        *self.running.write().await = true;

        let running = self.running.clone();
        let poll_interval = self.config.poll_interval;
        let reminders = self.reminders.clone();
        let notification_tx = self.notification_tx.clone();
        let quiet_start = self.config.quiet_start;
        let quiet_end = self.config.quiet_end;
        let enable_quiet = self.config.enable_quiet_hours;

        tokio::spawn(async move {
            info!("Scheduler started");

            while *running.read().await {
                // Check quiet hours
                let is_quiet = if enable_quiet {
                    let hour = chrono::Local::now().hour() as u8;
                    if quiet_start < quiet_end {
                        hour >= quiet_start && hour < quiet_end
                    } else {
                        hour >= quiet_start || hour < quiet_end
                    }
                } else {
                    false
                };

                if !is_quiet {
                    let now = chrono::Utc::now().timestamp();

                    // Find due reminders
                    let due: Vec<Reminder> = {
                        let r = reminders.read().await;
                        r.values()
                            .filter(|rem| rem.due_at <= now)
                            .cloned()
                            .collect()
                    };

                    for reminder in due {
                        let id = reminder.id.clone();

                        // Send notification
                        if notification_tx.send(reminder.clone()).await.is_err() {
                            warn!("Failed to send notification");
                            continue;
                        }

                        // Handle recurrence or remove
                        let mut reminders_guard = reminders.write().await;
                        if let Some(ref rule) = reminder.recurring {
                            if rule.has_more() {
                                let mut next_rule = rule.clone();
                                next_rule.occurrences += 1;
                                let mut next = reminder.clone();
                                next.id = uuid::Uuid::new_v4().to_string();
                                next.due_at = rule.next_from(next.due_at);
                                next.recurring = Some(next_rule);
                                reminders_guard.insert(next.id.clone(), next);
                            }
                        }
                        reminders_guard.remove(&id);
                    }
                }

                tokio::time::sleep(poll_interval).await;
            }

            info!("Scheduler stopped");
        });
    }

    /// Stop the scheduler
    pub async fn stop(&self) {
        *self.running.write().await = false;
    }

    /// Get scheduler stats
    pub async fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            pending_reminders: self.reminders.read().await.len(),
            pending_tasks: self.tasks.read().await.values().filter(|t| t.active).count(),
            is_running: *self.running.read().await,
        }
    }
}

/// Scheduler statistics
#[derive(Debug, Clone)]
pub struct SchedulerStats {
    pub pending_reminders: usize,
    pub pending_tasks: usize,
    pub is_running: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reminder_creation() {
        let reminder = Reminder::once(123, 456, "Test reminder", chrono::Utc::now().timestamp() + 60)
            .with_priority(Priority::High)
            .with_type(NotificationType::Reminder);

        assert_eq!(reminder.user_id, 123);
        assert_eq!(reminder.priority, Priority::High);
        assert!(!reminder.is_due());
    }

    #[test]
    fn test_recurrence_rule() {
        let rule = RecurrenceRule::daily().every(2).times(5);
        assert_eq!(rule.every, 2);
        assert_eq!(rule.max_occurrences, Some(5));
        assert!(rule.has_more());

        let now = chrono::Utc::now().timestamp();
        let next = rule.next_from(now);
        assert_eq!(next, now + 2 * 86400);
    }

    #[test]
    fn test_recurring_reminder() {
        let reminder = Reminder::once(1, 1, "Daily check", chrono::Utc::now().timestamp())
            .recurring(RecurrenceRule::daily());

        assert!(reminder.next_occurrence().is_some());
    }

    #[tokio::test]
    async fn test_scheduler() {
        let (scheduler, mut rx) = Scheduler::new(10);

        // Schedule a reminder due now
        let reminder = Reminder::once(1, 1, "Test", chrono::Utc::now().timestamp() - 1);
        scheduler.schedule_reminder(reminder).await;

        // Process
        let processed = scheduler.process_due().await;
        assert_eq!(processed, 1);

        // Should receive notification
        let notification = rx.try_recv();
        assert!(notification.is_ok());
    }

    #[tokio::test]
    async fn test_cancel_reminder() {
        let (scheduler, _rx) = Scheduler::new(10);

        let reminder = Reminder::once(1, 1, "Test", chrono::Utc::now().timestamp() + 3600);
        let id = scheduler.schedule_reminder(reminder).await;

        assert!(scheduler.cancel_reminder(&id).await);
        assert!(!scheduler.cancel_reminder(&id).await); // Already removed
    }

    #[tokio::test]
    async fn test_user_reminders() {
        let (scheduler, _rx) = Scheduler::new(10);

        scheduler.schedule_reminder(Reminder::once(1, 1, "R1", chrono::Utc::now().timestamp() + 100)).await;
        scheduler.schedule_reminder(Reminder::once(1, 1, "R2", chrono::Utc::now().timestamp() + 200)).await;
        scheduler.schedule_reminder(Reminder::once(2, 2, "R3", chrono::Utc::now().timestamp() + 300)).await;

        let user1_reminders = scheduler.get_user_reminders(1).await;
        assert_eq!(user1_reminders.len(), 2);
    }

    #[test]
    fn test_priority_ordering() {
        use std::collections::BinaryHeap;

        let low = QueueEntry { due_at: 100, priority: Priority::Low, id: "a".to_string() };
        let high = QueueEntry { due_at: 200, priority: Priority::High, id: "b".to_string() };

        // In a BinaryHeap, higher priority items should be popped first
        let mut heap = BinaryHeap::new();
        heap.push(low.clone());
        heap.push(high.clone());

        // High priority should be popped first
        assert_eq!(heap.pop().unwrap().priority, Priority::High);
        assert_eq!(heap.pop().unwrap().priority, Priority::Low);
    }

    #[test]
    fn test_scheduled_task() {
        let task = ScheduledTask::new("backup", "backup_db", chrono::Utc::now().timestamp() - 1)
            .with_callback("handle_backup");

        assert!(task.is_due());
        assert_eq!(task.callback, "handle_backup");
    }
}
