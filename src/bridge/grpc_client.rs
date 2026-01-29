//! gRPC Bridge Client
//!
//! Streaming gRPC client with TLS for connecting to the bridge server.

use anyhow::Result;
use std::path::PathBuf;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tracing::{debug, error, info};

use super::proto::{
    bridge_service_client::BridgeServiceClient, ExecuteChunk, ExecuteRequest, FileReadRequest,
    FileReadResponse, HealthRequest, StatusRequest, StatusResponse,
};

/// gRPC client configuration
#[derive(Debug, Clone)]
pub struct GrpcBridgeClientConfig {
    pub endpoint: String,
    pub api_key: String,
    pub timeout_seconds: u64,
    pub ca_cert_path: Option<PathBuf>,
    pub domain: Option<String>,
}

impl Default for GrpcBridgeClientConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:9998".to_string(),
            api_key: String::new(),
            timeout_seconds: 300,
            ca_cert_path: None,
            domain: None,
        }
    }
}

/// gRPC Bridge Client
///
/// Clone is cheap - tonic clients share the underlying connection.
#[derive(Clone)]
pub struct GrpcBridgeClient {
    client: BridgeServiceClient<Channel>,
    api_key: String,
}

impl GrpcBridgeClient {
    /// Create a new gRPC client
    pub async fn new(config: GrpcBridgeClientConfig) -> Result<Self> {
        let mut channel_builder = Channel::from_shared(config.endpoint.clone())?
            .timeout(std::time::Duration::from_secs(config.timeout_seconds));

        // Configure TLS if CA cert provided
        if let Some(ca_path) = &config.ca_cert_path {
            let ca_cert = tokio::fs::read(ca_path).await?;
            let ca = Certificate::from_pem(ca_cert);

            let mut tls = ClientTlsConfig::new().ca_certificate(ca);

            if let Some(domain) = &config.domain {
                tls = tls.domain_name(domain);
            }

            channel_builder = channel_builder.tls_config(tls)?;
            info!("TLS enabled for gRPC client");
        }

        let channel = channel_builder.connect().await?;
        let client = BridgeServiceClient::new(channel);

        Ok(Self {
            client,
            api_key: config.api_key,
        })
    }

    /// Create from environment variables
    pub async fn from_env() -> Result<Self> {
        let endpoint = std::env::var("BRIDGE_GRPC_URL")
            .map_err(|_| anyhow::anyhow!("BRIDGE_GRPC_URL environment variable required"))?;

        let api_key = std::env::var("BRIDGE_API_KEY")
            .map_err(|_| anyhow::anyhow!("BRIDGE_API_KEY environment variable required"))?;

        let timeout_seconds = std::env::var("BRIDGE_TIMEOUT")
            .unwrap_or_else(|_| "300".to_string())
            .parse()
            .unwrap_or(300);

        let ca_cert_path = std::env::var("BRIDGE_CA_CERT").ok().map(PathBuf::from);
        let domain = std::env::var("BRIDGE_TLS_DOMAIN").ok();

        let config = GrpcBridgeClientConfig {
            endpoint,
            api_key,
            timeout_seconds,
            ca_cert_path,
            domain,
        };

        Self::new(config).await
    }

    /// Add authorization header to request
    fn add_auth<T>(&self, mut request: tonic::Request<T>) -> tonic::Request<T> {
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", self.api_key).parse().unwrap(),
        );
        request
    }

    /// Health check
    pub async fn health_check(&self) -> Result<bool> {
        let request = tonic::Request::new(HealthRequest {});

        match self.client.clone().health(request).await {
            Ok(response) => {
                let status = response.into_inner();
                Ok(status.status == "healthy")
            }
            Err(e) => {
                debug!("Health check failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Get bridge status
    pub async fn status(&self) -> Result<StatusResponse> {
        let request = self.add_auth(tonic::Request::new(StatusRequest {}));

        let response = self.client.clone().status(request).await?;
        Ok(response.into_inner())
    }

    /// Execute a task with streaming response
    pub async fn execute(
        &self,
        req: ExecuteRequest,
    ) -> Result<tonic::Streaming<ExecuteChunk>> {
        debug!("gRPC Execute: {} chars", req.task.len());

        let request = self.add_auth(tonic::Request::new(req));

        let response = self.client.clone().execute(request).await?;
        Ok(response.into_inner())
    }

    /// Execute task and collect full response (convenience method)
    pub async fn execute_full(
        &self,
        chat_id: i64,
        task: &str,
        session_id: Option<String>,
    ) -> Result<ExecuteResult> {
        let req = ExecuteRequest {
            task: task.to_string(),
            session_id,
            working_dir: None,
            chat_id,
            autonomous: true,
        };

        let mut stream = self.execute(req).await?;
        let mut result = ExecuteResult::default();

        while let Some(chunk) = stream.message().await? {
            if !chunk.content.is_empty() {
                result.text.push_str(&chunk.content);
            }
            if chunk.session_id.is_some() {
                result.session_id = chunk.session_id;
            }
            if chunk.cost_usd.is_some() {
                result.cost_usd = chunk.cost_usd;
            }
            if chunk.duration_ms.is_some() {
                result.duration_ms = chunk.duration_ms.unwrap_or(0);
            }
            if chunk.error.is_some() {
                result.error = chunk.error;
                result.success = false;
            }
            if chunk.is_final {
                if result.error.is_none() {
                    result.success = true;
                }
                break;
            }
        }

        info!(
            "gRPC Execute completed: success={}, duration={}ms",
            result.success, result.duration_ms
        );

        Ok(result)
    }

    /// Read a file
    pub async fn read_file(&self, req: FileReadRequest) -> Result<FileReadResponse> {
        debug!("gRPC ReadFile: {}", req.path);

        let request = self.add_auth(tonic::Request::new(req));

        let response = self.client.clone().read_file(request).await?;
        let inner = response.into_inner();

        if inner.success {
            info!("File read successful: {} bytes", inner.file_size.unwrap_or(0));
        } else {
            error!("File read failed: {:?}", inner.error);
        }

        Ok(inner)
    }

    /// Read file with analysis
    pub async fn read_file_analyzed(&self, path: &str) -> Result<FileReadResponse> {
        let req = FileReadRequest {
            path: path.to_string(),
            analyze: true,
            max_bytes: 0,
        };
        self.read_file(req).await
    }

    /// Read raw file content
    pub async fn read_file_raw(&self, path: &str, max_bytes: u32) -> Result<FileReadResponse> {
        let req = FileReadRequest {
            path: path.to_string(),
            analyze: false,
            max_bytes,
        };
        self.read_file(req).await
    }

    /// Test connection
    pub async fn test_connection(&self) -> Result<String> {
        let healthy = self.health_check().await?;
        if !healthy {
            return Err(anyhow::anyhow!("Bridge server is not responding"));
        }

        let status = self.status().await?;
        Ok(format!(
            "gRPC Bridge v{} - {} requests processed, {} active sessions, uptime: {}s",
            status.version,
            status.requests_processed,
            status.active_sessions,
            status.uptime_seconds
        ))
    }
}

/// Result from execute_full
#[derive(Debug, Default)]
pub struct ExecuteResult {
    pub success: bool,
    pub text: String,
    pub session_id: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_default() {
        let config = GrpcBridgeClientConfig::default();
        assert_eq!(config.endpoint, "http://localhost:9998");
        assert_eq!(config.timeout_seconds, 300);
    }
}
