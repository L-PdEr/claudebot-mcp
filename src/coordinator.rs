//! Task Coordinator
//!
//! Coordinates task distribution across workers with:
//! - Task decomposition
//! - Worker assignment
//! - Progress tracking
//! - Circuit breaker for failure prevention

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

// ============ Circuit Breaker ============

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    Closed,    // Normal operation
    Open,      // Failures exceeded, blocking calls
    HalfOpen,  // Testing if service recovered
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub timeout: Duration,
    pub half_open_max_calls: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout: Duration::from_secs(30),
            half_open_max_calls: 3,
        }
    }
}

/// Circuit breaker for preventing cascade failures
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    half_open_calls: u32,
    last_failure: Option<Instant>,
    last_state_change: Instant,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            half_open_calls: 0,
            last_failure: None,
            last_state_change: Instant::now(),
        }
    }

    /// Check if the circuit allows calls
    pub fn can_call(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has passed
                if self.last_state_change.elapsed() >= self.config.timeout {
                    self.transition_to(CircuitState::HalfOpen);
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                self.half_open_calls < self.config.half_open_max_calls
            }
        }
    }

    /// Record a successful call
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.config.success_threshold {
                    self.transition_to(CircuitState::Closed);
                }
            }
            CircuitState::Open => {
                // Should not happen if can_call is used properly
            }
        }
    }

    /// Record a failed call
    pub fn record_failure(&mut self) {
        self.last_failure = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.config.failure_threshold {
                    self.transition_to(CircuitState::Open);
                }
            }
            CircuitState::HalfOpen => {
                // Single failure in half-open trips the breaker
                self.transition_to(CircuitState::Open);
            }
            CircuitState::Open => {
                // Already open, no change needed
            }
        }
    }

    /// Get current state
    pub fn state(&self) -> CircuitState {
        self.state
    }

    /// Get stats
    pub fn stats(&self) -> CircuitStats {
        CircuitStats {
            state: self.state,
            failure_count: self.failure_count,
            success_count: self.success_count,
            time_in_state: self.last_state_change.elapsed(),
            last_failure: self.last_failure.map(|f| f.elapsed()),
        }
    }

    fn transition_to(&mut self, state: CircuitState) {
        info!("Circuit breaker: {:?} -> {:?}", self.state, state);
        self.state = state;
        self.last_state_change = Instant::now();

        match state {
            CircuitState::Closed => {
                self.failure_count = 0;
                self.success_count = 0;
            }
            CircuitState::HalfOpen => {
                self.half_open_calls = 0;
                self.success_count = 0;
            }
            CircuitState::Open => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct CircuitStats {
    pub state: CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub time_in_state: Duration,
    pub last_failure: Option<Duration>,
}

// ============ Task Decomposition ============

/// Task priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Task status
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Queued,
    Assigned(String), // Worker ID
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

/// A task that can be decomposed into subtasks
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub parent_id: Option<String>,
    pub subtask_ids: Vec<String>,
    pub assigned_worker: Option<String>,
    pub created_at: Instant,
    pub started_at: Option<Instant>,
    pub completed_at: Option<Instant>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl Task {
    pub fn new(description: &str, priority: TaskPriority) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            description: description.to_string(),
            priority,
            status: TaskStatus::Pending,
            parent_id: None,
            subtask_ids: Vec::new(),
            assigned_worker: None,
            created_at: Instant::now(),
            started_at: None,
            completed_at: None,
            result: None,
            error: None,
            metadata: HashMap::new(),
        }
    }

    /// Check if task is ready to execute (no pending subtasks)
    pub fn is_ready(&self) -> bool {
        self.status == TaskStatus::Pending && self.subtask_ids.is_empty()
    }

    /// Check if task is done
    pub fn is_done(&self) -> bool {
        matches!(self.status, TaskStatus::Completed | TaskStatus::Failed(_) | TaskStatus::Cancelled)
    }

    /// Get task duration
    pub fn duration(&self) -> Option<Duration> {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => Some(end.duration_since(start)),
            (Some(start), None) => Some(start.elapsed()),
            _ => None,
        }
    }
}

/// Task decomposition hints
#[derive(Debug, Clone)]
pub struct DecompositionHint {
    pub task_type: TaskType,
    pub suggested_workers: u32,
    pub estimated_complexity: f64,
}

/// Types of tasks for routing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskType {
    Backend,
    Frontend,
    Tests,
    Infra,
    Documentation,
    Review,
    Generic,
}

impl TaskType {
    pub fn from_description(desc: &str) -> Self {
        let lower = desc.to_lowercase();

        if lower.contains("test") || lower.contains("spec") {
            TaskType::Tests
        } else if lower.contains("frontend") || lower.contains("ui") || lower.contains("react") || lower.contains("css") {
            TaskType::Frontend
        } else if lower.contains("backend") || lower.contains("api") || lower.contains("database") || lower.contains("server") {
            TaskType::Backend
        } else if lower.contains("deploy") || lower.contains("docker") || lower.contains("ci") || lower.contains("infra") {
            TaskType::Infra
        } else if lower.contains("doc") || lower.contains("readme") || lower.contains("comment") {
            TaskType::Documentation
        } else if lower.contains("review") || lower.contains("check") {
            TaskType::Review
        } else {
            TaskType::Generic
        }
    }
}

// ============ Task Queue ============

/// Dead letter queue entry
#[derive(Debug, Clone)]
pub struct DeadLetterEntry {
    pub task: Task,
    pub failure_reason: String,
    pub retry_count: u32,
    pub failed_at: Instant,
}

/// Task queue with priority ordering
pub struct TaskQueue {
    queues: [VecDeque<Task>; 4], // One queue per priority
    dead_letters: VecDeque<DeadLetterEntry>,
    max_retries: u32,
}

impl TaskQueue {
    pub fn new(max_retries: u32) -> Self {
        Self {
            queues: Default::default(),
            dead_letters: VecDeque::new(),
            max_retries,
        }
    }

    /// Enqueue a task
    pub fn enqueue(&mut self, mut task: Task) {
        task.status = TaskStatus::Queued;
        let priority = task.priority as usize;
        self.queues[priority].push_back(task);
    }

    /// Dequeue highest priority task
    pub fn dequeue(&mut self) -> Option<Task> {
        // Check from highest to lowest priority
        for priority in (0..4).rev() {
            if let Some(task) = self.queues[priority].pop_front() {
                return Some(task);
            }
        }
        None
    }

    /// Peek at next task without removing
    pub fn peek(&self) -> Option<&Task> {
        for priority in (0..4).rev() {
            if let Some(task) = self.queues[priority].front() {
                return Some(task);
            }
        }
        None
    }

    /// Send task to dead letter queue
    pub fn send_to_dlq(&mut self, task: Task, reason: &str, retry_count: u32) {
        if retry_count < self.max_retries {
            // Re-enqueue with incremented retry count
            let mut retry_task = task.clone();
            retry_task.status = TaskStatus::Pending;
            retry_task.metadata.insert("retry_count".to_string(), (retry_count + 1).to_string());
            self.enqueue(retry_task);
            debug!("Task {} re-queued (retry {})", task.id, retry_count + 1);
        } else {
            // Send to DLQ
            self.dead_letters.push_back(DeadLetterEntry {
                task,
                failure_reason: reason.to_string(),
                retry_count,
                failed_at: Instant::now(),
            });
            warn!("Task sent to DLQ after {} retries", retry_count);
        }
    }

    /// Get dead letters
    pub fn dead_letters(&self) -> &VecDeque<DeadLetterEntry> {
        &self.dead_letters
    }

    /// Queue length
    pub fn len(&self) -> usize {
        self.queues.iter().map(|q| q.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ============ Task Coordinator ============

/// Progress update for a task
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub task_id: String,
    pub percent: f64,
    pub message: String,
    pub timestamp: Instant,
}

/// Coordinator configuration
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub max_concurrent_tasks: usize,
    pub task_timeout: Duration,
    pub max_retries: u32,
    pub circuit_breaker: CircuitBreakerConfig,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 5,
            task_timeout: Duration::from_secs(300),
            max_retries: 3,
            circuit_breaker: CircuitBreakerConfig::default(),
        }
    }
}

/// Task coordinator
pub struct TaskCoordinator {
    config: CoordinatorConfig,
    tasks: Arc<RwLock<HashMap<String, Task>>>,
    queue: Arc<RwLock<TaskQueue>>,
    circuit_breaker: Arc<RwLock<CircuitBreaker>>,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    progress_rx: Arc<RwLock<mpsc::Receiver<ProgressUpdate>>>,
}

impl TaskCoordinator {
    pub fn new(config: CoordinatorConfig) -> Self {
        let (progress_tx, progress_rx) = mpsc::channel(100);

        Self {
            circuit_breaker: Arc::new(RwLock::new(CircuitBreaker::new(config.circuit_breaker.clone()))),
            queue: Arc::new(RwLock::new(TaskQueue::new(config.max_retries))),
            config,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            progress_tx,
            progress_rx: Arc::new(RwLock::new(progress_rx)),
        }
    }

    /// Submit a new task
    pub async fn submit(&self, description: &str, priority: TaskPriority) -> String {
        let task = Task::new(description, priority);
        let task_id = task.id.clone();

        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id.clone(), task.clone());

        let mut queue = self.queue.write().await;
        queue.enqueue(task);

        info!("Task submitted: {} (priority: {:?})", task_id, priority);
        task_id
    }

    /// Get next task for a worker
    pub async fn get_next_task(&self) -> Option<Task> {
        // Check circuit breaker
        let mut cb = self.circuit_breaker.write().await;
        if !cb.can_call() {
            warn!("Circuit breaker open, rejecting task request");
            return None;
        }

        let mut queue = self.queue.write().await;
        queue.dequeue()
    }

    /// Mark task as assigned to worker
    pub async fn assign_task(&self, task_id: &str, worker_id: &str) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Assigned(worker_id.to_string());
            task.assigned_worker = Some(worker_id.to_string());
        }
    }

    /// Mark task as running
    pub async fn start_task(&self, task_id: &str) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Running;
            task.started_at = Some(Instant::now());
        }
    }

    /// Complete a task successfully
    pub async fn complete_task(&self, task_id: &str, result: Option<String>) {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(Instant::now());
            task.result = result;

            let mut cb = self.circuit_breaker.write().await;
            cb.record_success();
        }
    }

    /// Fail a task
    pub async fn fail_task(&self, task_id: &str, error: &str) {
        let task = {
            let mut tasks = self.tasks.write().await;
            if let Some(task) = tasks.get_mut(task_id) {
                task.status = TaskStatus::Failed(error.to_string());
                task.completed_at = Some(Instant::now());
                task.error = Some(error.to_string());
                Some(task.clone())
            } else {
                None
            }
        };

        // Record failure and potentially send to DLQ
        if let Some(task) = task {
            let mut cb = self.circuit_breaker.write().await;
            cb.record_failure();

            let retry_count: u32 = task.metadata.get("retry_count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let mut queue = self.queue.write().await;
            queue.send_to_dlq(task, error, retry_count);
        }
    }

    /// Cancel a task
    pub async fn cancel_task(&self, task_id: &str) -> bool {
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            if !task.is_done() {
                task.status = TaskStatus::Cancelled;
                task.completed_at = Some(Instant::now());
                return true;
            }
        }
        false
    }

    /// Update task progress
    pub async fn update_progress(&self, task_id: &str, percent: f64, message: &str) {
        let _ = self.progress_tx.send(ProgressUpdate {
            task_id: task_id.to_string(),
            percent,
            message: message.to_string(),
            timestamp: Instant::now(),
        }).await;
    }

    /// Get task by ID
    pub async fn get_task(&self, task_id: &str) -> Option<Task> {
        let tasks = self.tasks.read().await;
        tasks.get(task_id).cloned()
    }

    /// Get all tasks
    pub async fn list_tasks(&self) -> Vec<Task> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// Get pending tasks count
    pub async fn pending_count(&self) -> usize {
        let queue = self.queue.read().await;
        queue.len()
    }

    /// Get circuit breaker stats
    pub async fn circuit_stats(&self) -> CircuitStats {
        let cb = self.circuit_breaker.read().await;
        cb.stats()
    }

    /// Decompose a complex task into subtasks
    pub fn decompose(&self, description: &str) -> Vec<Task> {
        let task_type = TaskType::from_description(description);
        let mut subtasks = Vec::new();

        // Simple decomposition based on keywords
        let lower = description.to_lowercase();

        if lower.contains(" and ") {
            // Split on "and"
            for part in description.split(" and ") {
                let part = part.trim();
                if !part.is_empty() {
                    subtasks.push(Task::new(part, TaskPriority::Normal));
                }
            }
        } else if lower.contains("then") {
            // Sequential tasks
            for (i, part) in description.split("then").enumerate() {
                let part = part.trim();
                if !part.is_empty() {
                    let mut task = Task::new(part, TaskPriority::Normal);
                    task.metadata.insert("sequence".to_string(), i.to_string());
                    subtasks.push(task);
                }
            }
        }

        // If no decomposition happened, return original as single task
        if subtasks.is_empty() {
            subtasks.push(Task::new(description, TaskPriority::Normal));
        }

        subtasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_closed() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        assert!(cb.can_call());
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        cb.record_failure();
        assert!(cb.can_call()); // Still closed

        cb.record_failure();
        assert!(!cb.can_call()); // Now open
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_recovery() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 2,
            timeout: Duration::from_millis(10),
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(15));

        assert!(cb.can_call()); // Should be half-open
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_task_queue_priority() {
        let mut queue = TaskQueue::new(3);

        queue.enqueue(Task::new("low", TaskPriority::Low));
        queue.enqueue(Task::new("high", TaskPriority::High));
        queue.enqueue(Task::new("normal", TaskPriority::Normal));

        let task = queue.dequeue().unwrap();
        assert!(task.description.contains("high"));

        let task = queue.dequeue().unwrap();
        assert!(task.description.contains("normal"));

        let task = queue.dequeue().unwrap();
        assert!(task.description.contains("low"));
    }

    #[test]
    fn test_task_type_detection() {
        assert_eq!(TaskType::from_description("write unit tests"), TaskType::Tests);
        assert_eq!(TaskType::from_description("update React component"), TaskType::Frontend);
        assert_eq!(TaskType::from_description("add API endpoint"), TaskType::Backend);
        assert_eq!(TaskType::from_description("configure Docker"), TaskType::Infra);
        assert_eq!(TaskType::from_description("random task"), TaskType::Generic);
    }

    #[tokio::test]
    async fn test_coordinator_submit() {
        let coordinator = TaskCoordinator::new(CoordinatorConfig::default());

        let task_id = coordinator.submit("Test task", TaskPriority::High).await;
        assert!(!task_id.is_empty());

        let task = coordinator.get_task(&task_id).await;
        assert!(task.is_some());
        assert_eq!(task.unwrap().priority, TaskPriority::High);
    }
}
