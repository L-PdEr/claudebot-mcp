//! Multi-Agent Orchestration System
//!
//! Enables spawning specialized sub-agents for complex tasks:
//! - Research Agent: Information gathering and analysis
//! - Code Agent: Programming and code generation
//! - Planning Agent: Task decomposition and scheduling
//! - Review Agent: Quality assurance and feedback
//!
//! Industry standard: Claude Code subagents, AutoGPT task delegation

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::info;

/// Types of specialized agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentType {
    /// Research and information gathering
    Research,
    /// Code generation and analysis
    Code,
    /// Task planning and decomposition
    Planning,
    /// Quality review and feedback
    Review,
    /// File and system operations
    FileOps,
    /// Web search and browsing
    Web,
    /// General purpose
    General,
}

impl AgentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Code => "code",
            Self::Planning => "planning",
            Self::Review => "review",
            Self::FileOps => "file_ops",
            Self::Web => "web",
            Self::General => "general",
        }
    }

    /// System prompt for this agent type
    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::Research => {
                "You are a Research Agent specialized in gathering and analyzing information. \
                 Focus on finding accurate, relevant data. Cite sources when possible. \
                 Be thorough but concise in your findings."
            }
            Self::Code => {
                "You are a Code Agent specialized in programming and software development. \
                 Write clean, efficient, well-documented code. Follow best practices. \
                 Include error handling and consider edge cases."
            }
            Self::Planning => {
                "You are a Planning Agent specialized in task decomposition and scheduling. \
                 Break complex tasks into actionable steps. Identify dependencies. \
                 Estimate effort and prioritize effectively."
            }
            Self::Review => {
                "You are a Review Agent specialized in quality assurance. \
                 Critically evaluate work for correctness, completeness, and quality. \
                 Provide constructive, specific feedback for improvement."
            }
            Self::FileOps => {
                "You are a File Operations Agent specialized in file and system tasks. \
                 Handle file operations carefully. Verify paths and permissions. \
                 Report clear results of operations."
            }
            Self::Web => {
                "You are a Web Agent specialized in web search and information retrieval. \
                 Find relevant, up-to-date information from the web. \
                 Summarize findings and provide source URLs."
            }
            Self::General => {
                "You are a helpful AI assistant. \
                 Provide clear, accurate, and helpful responses. \
                 Ask for clarification when needed."
            }
        }
    }
}

/// A task for a sub-agent to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub agent_type: AgentType,
    pub description: String,
    pub context: String,
    pub priority: u8,
    pub timeout: Duration,
    pub parent_task_id: Option<String>,
    pub dependencies: Vec<String>,
    pub created_at: i64,
}

impl AgentTask {
    /// Create a new agent task
    pub fn new(agent_type: AgentType, description: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            agent_type,
            description: description.to_string(),
            context: String::new(),
            priority: 5,
            timeout: Duration::from_secs(120),
            parent_task_id: None,
            dependencies: vec![],
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    /// Add context to the task
    pub fn with_context(mut self, context: &str) -> Self {
        self.context = context.to_string();
        self
    }

    /// Set priority (1-10, higher = more important)
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority.clamp(1, 10);
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set parent task
    pub fn with_parent(mut self, parent_id: &str) -> Self {
        self.parent_task_id = Some(parent_id.to_string());
        self
    }

    /// Add dependency
    pub fn depends_on(mut self, task_id: &str) -> Self {
        self.dependencies.push(task_id.to_string());
        self
    }
}

/// Status of an agent task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Result from an agent task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub task_id: String,
    pub agent_type: AgentType,
    pub status: TaskStatus,
    pub output: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub metadata: HashMap<String, String>,
}

impl AgentResult {
    /// Create a successful result
    pub fn success(task: &AgentTask, output: String, duration: Duration) -> Self {
        Self {
            task_id: task.id.clone(),
            agent_type: task.agent_type,
            status: TaskStatus::Completed,
            output,
            error: None,
            duration_ms: duration.as_millis() as u64,
            metadata: HashMap::new(),
        }
    }

    /// Create a failed result
    pub fn failure(task: &AgentTask, error: String, duration: Duration) -> Self {
        Self {
            task_id: task.id.clone(),
            agent_type: task.agent_type,
            status: TaskStatus::Failed,
            output: String::new(),
            error: Some(error),
            duration_ms: duration.as_millis() as u64,
            metadata: HashMap::new(),
        }
    }
}

/// A running sub-agent instance
pub struct SubAgent {
    pub id: String,
    pub agent_type: AgentType,
    pub task: AgentTask,
    pub started_at: Instant,
    status: Arc<RwLock<TaskStatus>>,
    result_tx: mpsc::Sender<AgentResult>,
}

impl SubAgent {
    /// Check if agent is still running
    pub async fn is_running(&self) -> bool {
        *self.status.read().await == TaskStatus::Running
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Check if timed out
    pub fn is_timed_out(&self) -> bool {
        self.elapsed() > self.task.timeout
    }
}

/// Configuration for orchestrator
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Maximum concurrent agents
    pub max_concurrent: usize,
    /// Default task timeout
    pub default_timeout: Duration,
    /// Enable parallel execution
    pub parallel_execution: bool,
    /// Maximum task queue size
    pub max_queue_size: usize,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 5,
            default_timeout: Duration::from_secs(120),
            parallel_execution: true,
            max_queue_size: 100,
        }
    }
}

/// Multi-agent orchestrator
pub struct AgentOrchestrator {
    config: OrchestratorConfig,
    /// Running agents
    agents: Arc<RwLock<HashMap<String, Arc<SubAgent>>>>,
    /// Completed results
    results: Arc<RwLock<HashMap<String, AgentResult>>>,
    /// Task queue
    queue: Arc<RwLock<Vec<AgentTask>>>,
    /// Result receiver
    result_rx: Arc<RwLock<mpsc::Receiver<AgentResult>>>,
    /// Result sender (for spawning)
    result_tx: mpsc::Sender<AgentResult>,
}

impl AgentOrchestrator {
    /// Create a new orchestrator
    pub fn new() -> Self {
        Self::with_config(OrchestratorConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: OrchestratorConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.max_queue_size);

        Self {
            config,
            agents: Arc::new(RwLock::new(HashMap::new())),
            results: Arc::new(RwLock::new(HashMap::new())),
            queue: Arc::new(RwLock::new(Vec::new())),
            result_rx: Arc::new(RwLock::new(rx)),
            result_tx: tx,
        }
    }

    /// Spawn a sub-agent for a task
    pub async fn spawn(&self, task: AgentTask) -> Result<String> {
        // Check queue size
        {
            let queue = self.queue.read().await;
            if queue.len() >= self.config.max_queue_size {
                return Err(anyhow::anyhow!("Task queue full"));
            }
        }

        // Check concurrent limit
        {
            let agents = self.agents.read().await;
            let running = agents.len();

            if running >= self.config.max_concurrent {
                // Queue the task
                self.queue.write().await.push(task.clone());
                info!("Task {} queued (at concurrent limit)", task.id);
                return Ok(task.id);
            }
        }

        // Create sub-agent
        let agent = Arc::new(SubAgent {
            id: uuid::Uuid::new_v4().to_string(),
            agent_type: task.agent_type,
            task: task.clone(),
            started_at: Instant::now(),
            status: Arc::new(RwLock::new(TaskStatus::Running)),
            result_tx: self.result_tx.clone(),
        });

        // Store agent
        self.agents.write().await.insert(task.id.clone(), Arc::clone(&agent));

        info!("Spawned {} agent for task {}", task.agent_type.as_str(), task.id);

        Ok(task.id)
    }

    /// Execute a task synchronously (for simple cases)
    pub async fn execute(
        &self,
        task: AgentTask,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<AgentResult> {
        let start = Instant::now();

        // Build prompt with system context
        let prompt = format!(
            "{}\n\nTask: {}\n\nContext:\n{}",
            task.agent_type.system_prompt(),
            task.description,
            task.context
        );

        // Execute with timeout
        let result = tokio::time::timeout(
            task.timeout,
            llama.generate(&prompt)
        ).await;

        match result {
            Ok(Ok(output)) => Ok(AgentResult::success(&task, output, start.elapsed())),
            Ok(Err(e)) => Ok(AgentResult::failure(&task, e.to_string(), start.elapsed())),
            Err(_) => Ok(AgentResult::failure(&task, "Task timed out".to_string(), start.elapsed())),
        }
    }

    /// Execute multiple tasks in parallel
    pub async fn execute_parallel(
        &self,
        tasks: Vec<AgentTask>,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Vec<AgentResult> {
        let mut results = Vec::with_capacity(tasks.len());

        // Execute tasks sequentially (parallel execution would require futures crate)
        for task in tasks {
            if let Ok(result) = self.execute(task, llama).await {
                results.push(result);
            }
        }

        results
    }

    /// Get result for a task
    pub async fn get_result(&self, task_id: &str) -> Option<AgentResult> {
        self.results.read().await.get(task_id).cloned()
    }

    /// Wait for a task to complete
    pub async fn wait_for(&self, task_id: &str, timeout: Duration) -> Option<AgentResult> {
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            if let Some(result) = self.get_result(task_id).await {
                return Some(result);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        None
    }

    /// Cancel a task
    pub async fn cancel(&self, task_id: &str) -> bool {
        if let Some(agent) = self.agents.write().await.remove(task_id) {
            *agent.status.write().await = TaskStatus::Cancelled;
            info!("Cancelled task {}", task_id);
            true
        } else {
            false
        }
    }

    /// Get statistics
    pub async fn stats(&self) -> OrchestratorStats {
        let agents = self.agents.read().await;
        let results = self.results.read().await;
        let queue = self.queue.read().await;

        let completed = results.values().filter(|r| r.status == TaskStatus::Completed).count();
        let failed = results.values().filter(|r| r.status == TaskStatus::Failed).count();

        OrchestratorStats {
            running: agents.len(),
            queued: queue.len(),
            completed,
            failed,
            total: results.len(),
        }
    }

    /// Decompose a complex task into sub-tasks
    pub async fn decompose(
        &self,
        description: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<Vec<AgentTask>> {
        let prompt = format!(
            r#"Decompose this task into sub-tasks for specialized agents.

Available agent types:
- research: Information gathering
- code: Programming tasks
- planning: Task organization
- review: Quality checking
- file_ops: File operations
- web: Web search

Task: {}

Return JSON array of sub-tasks:
[{{"agent": "research", "description": "...", "priority": 5}}, ...]

JSON only:"#,
            description
        );

        let response = llama.generate(&prompt).await?;

        // Parse response
        #[derive(Deserialize)]
        struct SubTask {
            agent: String,
            description: String,
            priority: Option<u8>,
        }

        let json_str = extract_json_array(&response).unwrap_or("[]");
        let sub_tasks: Vec<SubTask> = serde_json::from_str(json_str).unwrap_or_default();

        let tasks = sub_tasks
            .into_iter()
            .filter_map(|st| {
                let agent_type = match st.agent.as_str() {
                    "research" => AgentType::Research,
                    "code" => AgentType::Code,
                    "planning" => AgentType::Planning,
                    "review" => AgentType::Review,
                    "file_ops" => AgentType::FileOps,
                    "web" => AgentType::Web,
                    _ => AgentType::General,
                };

                Some(
                    AgentTask::new(agent_type, &st.description)
                        .with_priority(st.priority.unwrap_or(5))
                )
            })
            .collect();

        Ok(tasks)
    }
}

impl Default for AgentOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

/// Orchestrator statistics
#[derive(Debug, Clone, Default)]
pub struct OrchestratorStats {
    pub running: usize,
    pub queued: usize,
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
}

/// Extract JSON array from text
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let mut depth = 0;
    let mut end = start;

    for (i, c) in s[start..].char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if end > start {
        Some(&s[start..end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_task_creation() {
        let task = AgentTask::new(AgentType::Research, "Find information about Rust")
            .with_priority(8)
            .with_context("User is building a CLI tool");

        assert_eq!(task.agent_type, AgentType::Research);
        assert_eq!(task.priority, 8);
        assert!(!task.context.is_empty());
    }

    #[test]
    fn test_agent_type_system_prompts() {
        assert!(!AgentType::Research.system_prompt().is_empty());
        assert!(!AgentType::Code.system_prompt().is_empty());
        assert!(AgentType::Code.system_prompt().contains("programming"));
    }

    #[tokio::test]
    async fn test_orchestrator_stats() {
        let orchestrator = AgentOrchestrator::new();
        let stats = orchestrator.stats().await;

        assert_eq!(stats.running, 0);
        assert_eq!(stats.queued, 0);
    }

    #[test]
    fn test_extract_json_array() {
        let text = "Here are tasks: [{\"agent\": \"code\"}] done";
        assert_eq!(extract_json_array(text), Some("[{\"agent\": \"code\"}]"));
    }
}
