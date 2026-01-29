//! gRPC Bridge Server
//!
//! Streaming gRPC server with TLS for remote Claude Code execution.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, error, info, warn};

use super::proto::{
    bridge_service_server::{BridgeService, BridgeServiceServer},
    ChunkType, ExecuteChunk, ExecuteRequest, FileReadRequest, FileReadResponse,
    HealthRequest, HealthResponse, StatusRequest, StatusResponse,
};
use super::types::ClaudeCliOutput;

/// gRPC server configuration
#[derive(Debug, Clone)]
pub struct GrpcBridgeConfig {
    pub port: u16,
    pub api_key: String,
    pub working_dir: PathBuf,
    pub timeout_seconds: u64,
    pub rate_limit_per_minute: u32,
    pub allowed_admins: Vec<i64>,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
}

impl Default for GrpcBridgeConfig {
    fn default() -> Self {
        Self {
            port: 9998,
            api_key: String::new(),
            working_dir: PathBuf::from("/tmp/claudebot"),
            timeout_seconds: 300,
            rate_limit_per_minute: 10,
            allowed_admins: Vec::new(),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

/// Rate limit entry
struct RateLimitEntry {
    count: u32,
    window_start: Instant,
}

/// Shared state for the gRPC server
pub struct GrpcBridgeState {
    config: GrpcBridgeConfig,
    start_time: Instant,
    requests_processed: AtomicU64,
    sessions: RwLock<HashMap<i64, String>>,
    rate_limits: RwLock<HashMap<i64, RateLimitEntry>>,
}

impl GrpcBridgeState {
    pub fn new(config: GrpcBridgeConfig) -> Self {
        Self {
            config,
            start_time: Instant::now(),
            requests_processed: AtomicU64::new(0),
            sessions: RwLock::new(HashMap::new()),
            rate_limits: RwLock::new(HashMap::new()),
        }
    }

    async fn check_rate_limit(&self, chat_id: i64) -> bool {
        let mut limits = self.rate_limits.write().await;
        let now = Instant::now();
        let window = Duration::from_secs(60);

        let entry = limits.entry(chat_id).or_insert(RateLimitEntry {
            count: 0,
            window_start: now,
        });

        if now.duration_since(entry.window_start) >= window {
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count >= self.config.rate_limit_per_minute {
            return false;
        }

        entry.count += 1;
        true
    }

    fn is_admin(&self, chat_id: i64) -> bool {
        self.config.allowed_admins.is_empty() || self.config.allowed_admins.contains(&chat_id)
    }
}

/// gRPC Bridge Service implementation
pub struct GrpcBridgeServiceImpl {
    state: Arc<GrpcBridgeState>,
}

impl GrpcBridgeServiceImpl {
    pub fn new(state: Arc<GrpcBridgeState>) -> Self {
        Self { state }
    }
}

type ExecuteStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<ExecuteChunk, Status>> + Send>>;

#[tonic::async_trait]
impl BridgeService for GrpcBridgeServiceImpl {
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "healthy".to_string(),
            service: "claudebot-bridge-grpc".to_string(),
        }))
    }

    async fn status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let sessions = self.state.sessions.read().await;
        Ok(Response::new(StatusResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            healthy: true,
            requests_processed: self.state.requests_processed.load(Ordering::Relaxed),
            active_sessions: sessions.len() as u32,
            uptime_seconds: self.state.start_time.elapsed().as_secs(),
        }))
    }

    type ExecuteStream = ExecuteStream;

    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let req = request.into_inner();

        // Check admin access
        if !self.state.is_admin(req.chat_id) {
            warn!("Unauthorized chat_id: {}", req.chat_id);
            return Err(Status::permission_denied(format!(
                "Chat {} not authorized",
                req.chat_id
            )));
        }

        // Check rate limit
        if !self.state.check_rate_limit(req.chat_id).await {
            warn!("Rate limit exceeded for chat {}", req.chat_id);
            return Err(Status::resource_exhausted(format!(
                "Rate limit exceeded: max {} requests per minute",
                self.state.config.rate_limit_per_minute
            )));
        }

        self.state.requests_processed.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();

        info!(
            "gRPC Execute for chat {}: {} chars",
            req.chat_id,
            req.task.len()
        );

        // Get or create session
        let session_id = {
            let sessions = self.state.sessions.read().await;
            req.session_id.clone().or_else(|| sessions.get(&req.chat_id).cloned())
        };

        // Determine working directory
        let working_dir = req
            .working_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                self.state
                    .config
                    .working_dir
                    .join(format!("chat_{}", req.chat_id))
            });

        // Ensure directory exists
        if let Err(e) = std::fs::create_dir_all(&working_dir) {
            error!("Failed to create working directory: {}", e);
            return Err(Status::internal(format!(
                "Failed to create working directory: {}",
                e
            )));
        }

        let (tx, rx) = mpsc::channel(32);
        let state = self.state.clone();
        let timeout = Duration::from_secs(self.state.config.timeout_seconds);

        tokio::spawn(async move {
            let result = execute_and_stream(
                &tx,
                &req.task,
                session_id,
                &working_dir,
                req.autonomous,
                req.chat_id,
                timeout,
                start,
                state,
            )
            .await;

            if let Err(e) = result {
                let _ = tx
                    .send(Ok(ExecuteChunk {
                        r#type: ChunkType::Error as i32,
                        content: String::new(),
                        session_id: None,
                        cost_usd: None,
                        duration_ms: Some(start.elapsed().as_millis() as u64),
                        is_final: true,
                        error: Some(e.to_string()),
                    }))
                    .await;
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn read_file(
        &self,
        request: Request<FileReadRequest>,
    ) -> Result<Response<FileReadResponse>, Status> {
        let req = request.into_inner();

        info!("gRPC ReadFile: path={}", req.path);

        // Validate path
        let path = std::path::Path::new(&req.path);
        if !path.is_absolute() {
            return Ok(Response::new(FileReadResponse {
                success: false,
                content: String::new(),
                file_size: None,
                truncated: false,
                error: Some("Path must be absolute".to_string()),
            }));
        }

        if req.path.contains("..") || req.path.contains('\0') {
            warn!("Suspicious path rejected: {}", req.path);
            return Ok(Response::new(FileReadResponse {
                success: false,
                content: String::new(),
                file_size: None,
                truncated: false,
                error: Some("Invalid path".to_string()),
            }));
        }

        if !path.exists() {
            return Ok(Response::new(FileReadResponse {
                success: false,
                content: String::new(),
                file_size: None,
                truncated: false,
                error: Some(format!("File not found: {}", req.path)),
            }));
        }

        if !path.is_file() {
            return Ok(Response::new(FileReadResponse {
                success: false,
                content: String::new(),
                file_size: None,
                truncated: false,
                error: Some("Path is not a file".to_string()),
            }));
        }

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) => {
                return Ok(Response::new(FileReadResponse {
                    success: false,
                    content: String::new(),
                    file_size: None,
                    truncated: false,
                    error: Some(format!("Cannot read file metadata: {}", e)),
                }));
            }
        };
        let file_size = metadata.len();

        const MAX_FILE_SIZE: u64 = 1024 * 1024;
        let max_bytes = if req.max_bytes > 0 {
            req.max_bytes as u64
        } else {
            MAX_FILE_SIZE
        };

        let truncated = file_size > max_bytes;
        let bytes_to_read = std::cmp::min(file_size, max_bytes) as usize;

        match std::fs::read(path) {
            Ok(bytes) => {
                let content_bytes = if truncated {
                    &bytes[..bytes_to_read]
                } else {
                    &bytes[..]
                };

                let content = String::from_utf8(content_bytes.to_vec())
                    .unwrap_or_else(|_| String::from_utf8_lossy(content_bytes).to_string());

                Ok(Response::new(FileReadResponse {
                    success: true,
                    content,
                    file_size: Some(file_size),
                    truncated,
                    error: None,
                }))
            }
            Err(e) => Ok(Response::new(FileReadResponse {
                success: false,
                content: String::new(),
                file_size: Some(file_size),
                truncated: false,
                error: Some(format!("Failed to read file: {}", e)),
            })),
        }
    }
}

/// Execute Claude CLI and stream output chunks
async fn execute_and_stream(
    tx: &mpsc::Sender<Result<ExecuteChunk, Status>>,
    task: &str,
    session_id: Option<String>,
    working_dir: &PathBuf,
    autonomous: bool,
    chat_id: i64,
    timeout: Duration,
    start: Instant,
    state: Arc<GrpcBridgeState>,
) -> Result<()> {
    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(task)
        .arg("--verbose")
        .arg("--output-format")
        .arg("stream-json")
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(ref sid) = session_id {
        cmd.arg("--resume").arg(sid);
    }

    if autonomous {
        cmd.arg("--dangerously-skip-permissions");
    }

    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;

    let mut reader = BufReader::new(stdout).lines();
    let mut result_text = String::new();
    let mut final_session_id = session_id;
    let mut cost_usd = None;

    let stream_result = tokio::time::timeout(timeout, async {
        while let Some(line) = reader.next_line().await? {
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<ClaudeCliOutput>(&line) {
                Ok(output) => {
                    debug!("Claude output type: {}", output.output_type);

                    let chunk = match output.output_type.as_str() {
                        "result" => {
                            if let Some(text) = output.result {
                                result_text = text.clone();
                            }
                            if let Some(sid) = output.session_id.clone() {
                                final_session_id = Some(sid);
                            }
                            if let Some(c) = output.cost_usd {
                                cost_usd = Some(c);
                            }

                            ExecuteChunk {
                                r#type: ChunkType::Result as i32,
                                content: result_text.clone(),
                                session_id: final_session_id.clone(),
                                cost_usd,
                                duration_ms: Some(start.elapsed().as_millis() as u64),
                                is_final: true,
                                error: None,
                            }
                        }
                        "assistant" => {
                            let content = output.message.unwrap_or_default();
                            ExecuteChunk {
                                r#type: ChunkType::Assistant as i32,
                                content,
                                session_id: None,
                                cost_usd: None,
                                duration_ms: None,
                                is_final: false,
                                error: None,
                            }
                        }
                        "system" => {
                            if let Some(sid) = output.session_id.clone() {
                                final_session_id = Some(sid.clone());
                            }
                            ExecuteChunk {
                                r#type: ChunkType::System as i32,
                                content: String::new(),
                                session_id: output.session_id,
                                cost_usd: None,
                                duration_ms: None,
                                is_final: false,
                                error: None,
                            }
                        }
                        "error" => {
                            let err_msg = output.message.unwrap_or_else(|| "Unknown error".to_string());
                            ExecuteChunk {
                                r#type: ChunkType::Error as i32,
                                content: String::new(),
                                session_id: None,
                                cost_usd: None,
                                duration_ms: Some(start.elapsed().as_millis() as u64),
                                is_final: true,
                                error: Some(err_msg),
                            }
                        }
                        _ => continue,
                    };

                    let is_final = chunk.is_final;
                    if tx.send(Ok(chunk)).await.is_err() {
                        break; // Client disconnected
                    }
                    if is_final {
                        break;
                    }
                }
                Err(e) => {
                    debug!("Non-JSON line: {} (error: {})", line, e);
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;

    // Store session for future resumption
    if let Some(ref sid) = final_session_id {
        let mut sessions = state.sessions.write().await;
        sessions.insert(chat_id, sid.clone());
    }

    match stream_result {
        Ok(Ok(())) => {
            info!(
                "gRPC Execute completed for chat {} in {}ms",
                chat_id,
                start.elapsed().as_millis()
            );
        }
        Ok(Err(e)) => {
            error!("Claude execution failed: {}", e);
            return Err(e);
        }
        Err(_) => {
            error!(
                "Claude execution timed out after {}s",
                timeout.as_secs()
            );
            return Err(anyhow::anyhow!(
                "Execution timed out after {} seconds",
                timeout.as_secs()
            ));
        }
    }

    Ok(())
}

/// gRPC Bridge Server
pub struct GrpcBridgeServer {
    state: Arc<GrpcBridgeState>,
}

impl GrpcBridgeServer {
    pub fn new(config: GrpcBridgeConfig) -> Self {
        Self {
            state: Arc::new(GrpcBridgeState::new(config)),
        }
    }

    pub fn from_env() -> Result<Self> {
        let port = std::env::var("BRIDGE_GRPC_PORT")
            .unwrap_or_else(|_| "9998".to_string())
            .parse()
            .unwrap_or(9998);

        let api_key = std::env::var("BRIDGE_API_KEY")
            .map_err(|_| anyhow::anyhow!("BRIDGE_API_KEY environment variable required"))?;

        let working_dir = std::env::var("BRIDGE_WORKING_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/claudebot"));

        let timeout_seconds = std::env::var("BRIDGE_TIMEOUT")
            .unwrap_or_else(|_| "300".to_string())
            .parse()
            .unwrap_or(300);

        let rate_limit_per_minute = std::env::var("BRIDGE_RATE_LIMIT")
            .unwrap_or_else(|_| "10".to_string())
            .parse()
            .unwrap_or(10);

        let allowed_admins: Vec<i64> = std::env::var("BRIDGE_ALLOWED_ADMINS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        let tls_cert_path = std::env::var("BRIDGE_TLS_CERT").ok().map(PathBuf::from);
        let tls_key_path = std::env::var("BRIDGE_TLS_KEY").ok().map(PathBuf::from);

        let config = GrpcBridgeConfig {
            port,
            api_key,
            working_dir,
            timeout_seconds,
            rate_limit_per_minute,
            allowed_admins,
            tls_cert_path,
            tls_key_path,
        };

        Ok(Self::new(config))
    }

    pub async fn run(&self) -> Result<()> {
        std::fs::create_dir_all(&self.state.config.working_dir)?;

        let service = GrpcBridgeServiceImpl::new(self.state.clone());
        let addr = format!("0.0.0.0:{}", self.state.config.port).parse()?;

        info!("gRPC Bridge server starting on {}", addr);

        let mut builder = Server::builder();

        // Configure TLS if certs provided
        if let (Some(cert_path), Some(key_path)) = (
            &self.state.config.tls_cert_path,
            &self.state.config.tls_key_path,
        ) {
            let cert = tokio::fs::read(cert_path).await?;
            let key = tokio::fs::read(key_path).await?;
            let identity = Identity::from_pem(cert, key);

            builder = builder.tls_config(ServerTlsConfig::new().identity(identity))?;
            info!("TLS enabled for gRPC server");
        }

        builder
            .add_service(BridgeServiceServer::new(service))
            .serve(addr)
            .await?;

        Ok(())
    }

    pub fn port(&self) -> u16 {
        self.state.config.port
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_config_default() {
        let config = GrpcBridgeConfig::default();
        assert_eq!(config.port, 9998);
        assert_eq!(config.timeout_seconds, 300);
        assert_eq!(config.rate_limit_per_minute, 10);
    }

    #[test]
    fn test_grpc_state_creation() {
        let config = GrpcBridgeConfig {
            port: 8080,
            api_key: "test-key".to_string(),
            ..Default::default()
        };
        let state = GrpcBridgeState::new(config);
        assert_eq!(state.requests_processed.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_grpc_rate_limiting() {
        let config = GrpcBridgeConfig {
            rate_limit_per_minute: 3,
            ..Default::default()
        };
        let state = GrpcBridgeState::new(config);

        assert!(state.check_rate_limit(12345).await);
        assert!(state.check_rate_limit(12345).await);
        assert!(state.check_rate_limit(12345).await);
        assert!(!state.check_rate_limit(12345).await);
        assert!(state.check_rate_limit(99999).await);
    }

    #[test]
    fn test_grpc_admin_check() {
        let config = GrpcBridgeConfig {
            allowed_admins: vec![111, 222],
            ..Default::default()
        };
        let state = GrpcBridgeState::new(config);

        assert!(state.is_admin(111));
        assert!(state.is_admin(222));
        assert!(!state.is_admin(333));
    }
}
