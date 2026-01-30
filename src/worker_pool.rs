//! Worker Pool Manager
//!
//! Manages concurrent Claude Code workers with health monitoring,
//! automatic restart, and resource management.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Worker status
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerStatus {
    Starting,
    Idle,
    Busy,
    Failed(String),
    Stopped,
}

/// Permission level for workers
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum PermissionLevel {
    /// Read-only access
    Sandbox = 0,
    /// Read/write project, git to feature branches
    Standard = 1,
    /// Install packages, run builds, access .env
    Elevated = 2,
    /// System commands, network ops, credentials
    Bypass = 3,
    /// Root access - requires explicit confirmation
    Root = 4,
}

impl PermissionLevel {
    pub fn from_u8(level: u8) -> Self {
        match level {
            0 => Self::Sandbox,
            1 => Self::Standard,
            2 => Self::Elevated,
            3 => Self::Bypass,
            4 => Self::Root,
            _ => Self::Sandbox,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Standard => "standard",
            Self::Elevated => "elevated",
            Self::Bypass => "bypass",
            Self::Root => "root",
        }
    }

    /// Check if this level allows the given operation
    pub fn allows(&self, operation: &WorkerOperation) -> bool {
        match (self, operation) {
            // Sandbox: read only
            (Self::Sandbox, WorkerOperation::Read) => true,
            (Self::Sandbox, _) => false,

            // Standard: read/write, git branches
            (Self::Standard, WorkerOperation::Read) => true,
            (Self::Standard, WorkerOperation::Write) => true,
            (Self::Standard, WorkerOperation::GitBranch) => true,
            (Self::Standard, _) => false,

            // Elevated: + packages, builds, env
            (Self::Elevated, WorkerOperation::InstallPackage) => true,
            (Self::Elevated, WorkerOperation::RunBuild) => true,
            (Self::Elevated, WorkerOperation::AccessEnv) => true,
            (Self::Elevated, op) => Self::Standard.allows(op),

            // Bypass: + system, network, credentials
            (Self::Bypass, WorkerOperation::SystemCommand) => true,
            (Self::Bypass, WorkerOperation::NetworkOp) => true,
            (Self::Bypass, WorkerOperation::AccessCredentials) => true,
            (Self::Bypass, op) => Self::Elevated.allows(op),

            // Root: everything
            (Self::Root, _) => true,
        }
    }
}

/// Operations that workers can perform
#[derive(Debug, Clone)]
pub enum WorkerOperation {
    Read,
    Write,
    GitBranch,
    GitPush,
    InstallPackage,
    RunBuild,
    AccessEnv,
    SystemCommand,
    NetworkOp,
    AccessCredentials,
}

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub id: String,
    pub name: String,
    pub working_dir: PathBuf,
    pub permission_level: PermissionLevel,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: "worker".to_string(),
            working_dir: PathBuf::from("/home/eliot/workspace"),
            permission_level: PermissionLevel::Standard,
            timeout: Duration::from_secs(300),
            max_output_bytes: 10 * 1024 * 1024, // 10MB
        }
    }
}

/// A single worker instance
pub struct Worker {
    pub config: WorkerConfig,
    pub status: WorkerStatus,
    pub started_at: Option<Instant>,
    pub last_activity: Option<Instant>,
    pub task_count: u64,
    pub error_count: u64,
    process: Option<Child>,
}

impl Worker {
    pub fn new(config: WorkerConfig) -> Self {
        Self {
            config,
            status: WorkerStatus::Starting,
            started_at: None,
            last_activity: None,
            task_count: 0,
            error_count: 0,
            process: None,
        }
    }

    /// Execute a task on this worker
    pub async fn execute(&mut self, task: &str) -> Result<WorkerResult, WorkerError> {
        if self.status != WorkerStatus::Idle {
            return Err(WorkerError::NotIdle);
        }

        self.status = WorkerStatus::Busy;
        self.last_activity = Some(Instant::now());
        let start = Instant::now();

        // Build Claude CLI command
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(task)
            .arg("--output-format")
            .arg("json");

        // Apply permission level restrictions
        if self.config.permission_level >= PermissionLevel::Elevated {
            cmd.arg("--dangerously-skip-permissions");
        }

        cmd.current_dir(&self.config.working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| WorkerError::SpawnFailed(e.to_string()))?;

        // Read output with timeout
        let stdout = child.stdout.take();
        let _stderr = child.stderr.take();

        let mut all_output = String::new();
        let timeout = self.config.timeout;

        // Monitor output
        let result = tokio::time::timeout(timeout, async {
            if let Some(stdout) = stdout {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    all_output.push_str(&line);
                    all_output.push('\n');
                    if all_output.len() > self.config.max_output_bytes {
                        break;
                    }
                }
            }
            child.wait().await
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok(exit_status)) => {
                self.task_count += 1;
                self.status = WorkerStatus::Idle;
                self.last_activity = Some(Instant::now());

                if exit_status.success() {
                    Ok(WorkerResult {
                        success: true,
                        output: all_output,
                        error: None,
                        duration,
                        exit_code: Some(0),
                    })
                } else {
                    self.error_count += 1;
                    Ok(WorkerResult {
                        success: false,
                        output: all_output,
                        error: Some("Non-zero exit code".to_string()),
                        duration,
                        exit_code: exit_status.code(),
                    })
                }
            }
            Ok(Err(e)) => {
                self.error_count += 1;
                self.status = WorkerStatus::Failed(e.to_string());
                Err(WorkerError::ExecutionFailed(e.to_string()))
            }
            Err(_) => {
                // Timeout
                let _ = child.kill().await;
                self.error_count += 1;
                self.status = WorkerStatus::Idle;
                Err(WorkerError::Timeout)
            }
        }
    }

    /// Check if worker is healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self.status, WorkerStatus::Idle | WorkerStatus::Busy)
    }

    /// Get worker info
    pub fn info(&self) -> WorkerInfo {
        WorkerInfo {
            id: self.config.id.clone(),
            name: self.config.name.clone(),
            status: self.status.clone(),
            permission_level: self.config.permission_level,
            task_count: self.task_count,
            error_count: self.error_count,
            uptime_secs: self.started_at.map(|s| s.elapsed().as_secs()).unwrap_or(0),
            idle_secs: self.last_activity.map(|l| l.elapsed().as_secs()).unwrap_or(0),
        }
    }
}

/// Worker execution result
#[derive(Debug, Clone)]
pub struct WorkerResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub duration: Duration,
    pub exit_code: Option<i32>,
}

/// Worker info for reporting
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: String,
    pub name: String,
    pub status: WorkerStatus,
    pub permission_level: PermissionLevel,
    pub task_count: u64,
    pub error_count: u64,
    pub uptime_secs: u64,
    pub idle_secs: u64,
}

/// Worker errors
#[derive(Debug, Clone)]
pub enum WorkerError {
    NotIdle,
    SpawnFailed(String),
    ExecutionFailed(String),
    Timeout,
    PermissionDenied,
    WorkerNotFound,
    PoolFull,
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotIdle => write!(f, "Worker is not idle"),
            Self::SpawnFailed(e) => write!(f, "Failed to spawn worker: {}", e),
            Self::ExecutionFailed(e) => write!(f, "Execution failed: {}", e),
            Self::Timeout => write!(f, "Worker timed out"),
            Self::PermissionDenied => write!(f, "Permission denied"),
            Self::WorkerNotFound => write!(f, "Worker not found"),
            Self::PoolFull => write!(f, "Worker pool is full"),
        }
    }
}

impl std::error::Error for WorkerError {}

/// Worker pool configuration
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_workers: usize,
    pub default_timeout: Duration,
    pub health_check_interval: Duration,
    pub max_idle_time: Duration,
    pub restart_on_failure: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_workers: 5,
            default_timeout: Duration::from_secs(300),
            health_check_interval: Duration::from_secs(30),
            max_idle_time: Duration::from_secs(600),
            restart_on_failure: true,
        }
    }
}

/// Worker pool manager
pub struct WorkerPool {
    config: PoolConfig,
    workers: Arc<RwLock<HashMap<String, Arc<Mutex<Worker>>>>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl WorkerPool {
    /// Create a new worker pool
    pub fn new(config: PoolConfig) -> Self {
        Self {
            config,
            workers: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx: None,
        }
    }

    /// Start the worker pool with background health monitoring
    pub async fn start(&mut self) {
        let (tx, mut rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(tx);

        let workers = Arc::clone(&self.workers);
        let interval = self.config.health_check_interval;
        let max_idle = self.config.max_idle_time;
        let restart_on_failure = self.config.restart_on_failure;

        // Spawn health monitor
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        Self::health_check(&workers, max_idle, restart_on_failure).await;
                    }
                    _ = rx.recv() => {
                        info!("Worker pool shutting down");
                        break;
                    }
                }
            }
        });

        info!("Worker pool started with max {} workers", self.config.max_workers);
    }

    /// Stop the worker pool
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        // Kill all workers
        let workers = self.workers.read().await;
        for worker in workers.values() {
            let mut w = worker.lock().await;
            w.status = WorkerStatus::Stopped;
        }
        info!("Worker pool stopped");
    }

    /// Spawn a new worker
    pub async fn spawn_worker(&self, config: WorkerConfig) -> Result<String, WorkerError> {
        let workers = self.workers.read().await;
        if workers.len() >= self.config.max_workers {
            return Err(WorkerError::PoolFull);
        }
        drop(workers);

        let worker_id = config.id.clone();
        let mut worker = Worker::new(config);
        worker.status = WorkerStatus::Idle;
        worker.started_at = Some(Instant::now());

        let mut workers = self.workers.write().await;
        workers.insert(worker_id.clone(), Arc::new(Mutex::new(worker)));

        info!("Spawned worker: {}", worker_id);
        Ok(worker_id)
    }

    /// Get a worker by ID
    pub async fn get_worker(&self, id: &str) -> Option<Arc<Mutex<Worker>>> {
        let workers = self.workers.read().await;
        workers.get(id).cloned()
    }

    /// Execute a task on an available worker
    pub async fn execute(&self, task: &str, preferred_worker: Option<&str>) -> Result<WorkerResult, WorkerError> {
        let worker = if let Some(id) = preferred_worker {
            self.get_worker(id).await.ok_or(WorkerError::WorkerNotFound)?
        } else {
            self.get_idle_worker().await.ok_or(WorkerError::NotIdle)?
        };

        let mut w = worker.lock().await;
        w.execute(task).await
    }

    /// Get an idle worker
    pub async fn get_idle_worker(&self) -> Option<Arc<Mutex<Worker>>> {
        let workers = self.workers.read().await;
        for worker in workers.values() {
            let w = worker.lock().await;
            if w.status == WorkerStatus::Idle {
                return Some(Arc::clone(worker));
            }
        }
        None
    }

    /// Kill a worker
    pub async fn kill_worker(&self, id: &str) -> Result<(), WorkerError> {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.remove(id) {
            let mut w = worker.lock().await;
            w.status = WorkerStatus::Stopped;
            if let Some(mut process) = w.process.take() {
                let _ = process.kill().await;
            }
            info!("Killed worker: {}", id);
            Ok(())
        } else {
            Err(WorkerError::WorkerNotFound)
        }
    }

    /// List all workers
    pub async fn list_workers(&self) -> Vec<WorkerInfo> {
        let workers = self.workers.read().await;
        let mut infos = Vec::new();
        for worker in workers.values() {
            let w = worker.lock().await;
            infos.push(w.info());
        }
        infos
    }

    /// Get pool statistics
    pub async fn stats(&self) -> PoolStats {
        let workers = self.workers.read().await;
        let mut stats = PoolStats {
            total_workers: workers.len(),
            idle_workers: 0,
            busy_workers: 0,
            failed_workers: 0,
            total_tasks: 0,
            total_errors: 0,
        };

        for worker in workers.values() {
            let w = worker.lock().await;
            match w.status {
                WorkerStatus::Idle => stats.idle_workers += 1,
                WorkerStatus::Busy => stats.busy_workers += 1,
                WorkerStatus::Failed(_) => stats.failed_workers += 1,
                _ => {}
            }
            stats.total_tasks += w.task_count;
            stats.total_errors += w.error_count;
        }

        stats
    }

    /// Background health check
    async fn health_check(
        workers: &Arc<RwLock<HashMap<String, Arc<Mutex<Worker>>>>>,
        max_idle: Duration,
        restart_on_failure: bool,
    ) {
        let workers_read = workers.read().await;
        let mut to_remove = Vec::new();
        let mut to_restart = Vec::new();

        for (id, worker) in workers_read.iter() {
            let w = worker.lock().await;

            // Check for idle timeout
            if let Some(last) = w.last_activity {
                if last.elapsed() > max_idle && w.status == WorkerStatus::Idle {
                    debug!("Worker {} idle too long, marking for removal", id);
                    to_remove.push(id.clone());
                }
            }

            // Check for failures
            if let WorkerStatus::Failed(ref reason) = w.status {
                if restart_on_failure {
                    warn!("Worker {} failed: {}, marking for restart", id, reason);
                    to_restart.push((id.clone(), w.config.clone()));
                } else {
                    to_remove.push(id.clone());
                }
            }
        }
        drop(workers_read);

        // Remove workers
        if !to_remove.is_empty() {
            let mut workers_write = workers.write().await;
            for id in to_remove {
                workers_write.remove(&id);
                info!("Removed worker: {}", id);
            }
        }

        // Restart workers
        for (old_id, config) in to_restart {
            let mut workers_write = workers.write().await;
            workers_write.remove(&old_id);

            let new_config = WorkerConfig {
                id: Uuid::new_v4().to_string(),
                ..config
            };
            let mut worker = Worker::new(new_config.clone());
            worker.status = WorkerStatus::Idle;
            worker.started_at = Some(Instant::now());
            workers_write.insert(new_config.id.clone(), Arc::new(Mutex::new(worker)));
            info!("Restarted worker {} as {}", old_id, new_config.id);
        }
    }
}

/// Pool statistics
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    pub total_workers: usize,
    pub idle_workers: usize,
    pub busy_workers: usize,
    pub failed_workers: usize,
    pub total_tasks: u64,
    pub total_errors: u64,
}

impl PoolStats {
    pub fn utilization(&self) -> f64 {
        if self.total_workers == 0 {
            return 0.0;
        }
        self.busy_workers as f64 / self.total_workers as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_levels() {
        let sandbox = PermissionLevel::Sandbox;
        let standard = PermissionLevel::Standard;
        let elevated = PermissionLevel::Elevated;
        let bypass = PermissionLevel::Bypass;
        let root = PermissionLevel::Root;

        // Sandbox can only read
        assert!(sandbox.allows(&WorkerOperation::Read));
        assert!(!sandbox.allows(&WorkerOperation::Write));

        // Standard can read/write and git branch
        assert!(standard.allows(&WorkerOperation::Read));
        assert!(standard.allows(&WorkerOperation::Write));
        assert!(standard.allows(&WorkerOperation::GitBranch));
        assert!(!standard.allows(&WorkerOperation::InstallPackage));

        // Elevated adds package install, builds, env
        assert!(elevated.allows(&WorkerOperation::InstallPackage));
        assert!(elevated.allows(&WorkerOperation::RunBuild));
        assert!(elevated.allows(&WorkerOperation::AccessEnv));
        assert!(!elevated.allows(&WorkerOperation::SystemCommand));

        // Bypass adds system, network, credentials
        assert!(bypass.allows(&WorkerOperation::SystemCommand));
        assert!(bypass.allows(&WorkerOperation::NetworkOp));
        assert!(bypass.allows(&WorkerOperation::AccessCredentials));

        // Root allows everything
        assert!(root.allows(&WorkerOperation::Read));
        assert!(root.allows(&WorkerOperation::SystemCommand));
        assert!(root.allows(&WorkerOperation::AccessCredentials));
    }

    #[tokio::test]
    async fn test_worker_pool_creation() {
        let pool = WorkerPool::new(PoolConfig::default());
        let stats = pool.stats().await;
        assert_eq!(stats.total_workers, 0);
    }

    #[tokio::test]
    async fn test_spawn_worker() {
        let pool = WorkerPool::new(PoolConfig::default());

        let config = WorkerConfig {
            name: "test-worker".to_string(),
            ..Default::default()
        };

        let id = pool.spawn_worker(config).await.unwrap();
        assert!(!id.is_empty());

        let stats = pool.stats().await;
        assert_eq!(stats.total_workers, 1);
        assert_eq!(stats.idle_workers, 1);
    }

    #[tokio::test]
    async fn test_pool_max_workers() {
        let config = PoolConfig {
            max_workers: 2,
            ..Default::default()
        };
        let pool = WorkerPool::new(config);

        // Spawn up to max
        pool.spawn_worker(WorkerConfig::default()).await.unwrap();
        pool.spawn_worker(WorkerConfig::default()).await.unwrap();

        // Third should fail
        let result = pool.spawn_worker(WorkerConfig::default()).await;
        assert!(matches!(result, Err(WorkerError::PoolFull)));
    }
}
