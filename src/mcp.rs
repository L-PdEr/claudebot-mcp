//! MCP Protocol Handler
//!
//! Implements JSON-RPC 2.0 over stdio for Model Context Protocol.
//! Reference: https://modelcontextprotocol.io/specification

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::tools::ToolRegistry;

/// JSON-RPC 2.0 Request
#[derive(Debug, Clone, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Clone, Serialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
    pub id: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 Error
#[derive(Debug, Clone, Serialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl McpResponse {
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(McpError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    /// Notification (no id, no response expected)
    pub fn notification() -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: None,
            id: None,
        }
    }
}

/// MCP Error Codes
pub mod error_codes {
    // JSON-RPC standard errors
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    // MCP custom errors (-32000 to -32099)
    pub const TOOL_NOT_FOUND: i32 = -32000;
    pub const TOOL_EXECUTION_ERROR: i32 = -32001;
    pub const RESOURCE_NOT_FOUND: i32 = -32002;
}

/// Server capabilities
#[derive(Debug, Clone, Serialize)]
pub struct ServerCapabilities {
    pub tools: Option<ToolCapabilities>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCapabilities {
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

/// Server info
#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// MCP Server
pub struct McpServer {
    #[allow(dead_code)]
    config: Arc<Config>,
    tools: Arc<tokio::sync::Mutex<ToolRegistry>>,
}

impl McpServer {
    /// Create new MCP server
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let config = Arc::new(config);
        let tools = Arc::new(tokio::sync::Mutex::new(ToolRegistry::new(config.clone()).await?));

        Ok(Self { config, tools })
    }

    /// Run the MCP server (stdio mode)
    pub async fn run(&self) -> anyhow::Result<()> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        info!("MCP server ready, waiting for requests...");

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await?;

            if bytes_read == 0 {
                info!("Client disconnected (EOF)");
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            debug!("← {}", trimmed);

            let response = match serde_json::from_str::<McpRequest>(trimmed) {
                Ok(request) => {
                    // Handle notification (no id) - no response needed
                    if request.id.is_none() && request.method == "notifications/initialized" {
                        debug!("Received initialized notification");
                        continue;
                    }
                    self.handle_request(request).await
                }
                Err(e) => {
                    error!("Parse error: {}", e);
                    McpResponse::error(None, error_codes::PARSE_ERROR, format!("Parse error: {}", e))
                }
            };

            // Don't send response for notifications
            if response.id.is_none() && response.result.is_none() && response.error.is_none() {
                continue;
            }

            let response_json = serde_json::to_string(&response)?;
            debug!("→ {}", response_json);

            stdout.write_all(response_json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        Ok(())
    }

    /// Handle a single MCP request
    async fn handle_request(&self, request: McpRequest) -> McpResponse {
        match request.method.as_str() {
            // Lifecycle
            "initialize" => self.handle_initialize(request.id),
            "initialized" => McpResponse::notification(),
            "shutdown" => {
                info!("Shutdown requested");
                McpResponse::success(request.id, serde_json::json!({}))
            }

            // Tools
            "tools/list" => self.handle_tools_list(request.id).await,
            "tools/call" => self.handle_tools_call(request.id, request.params).await,

            // Ping
            "ping" => McpResponse::success(request.id, serde_json::json!({})),

            // Unknown
            method => {
                warn!("Unknown method: {}", method);
                McpResponse::error(
                    request.id,
                    error_codes::METHOD_NOT_FOUND,
                    format!("Method not found: {}", method),
                )
            }
        }
    }

    /// Handle initialize
    fn handle_initialize(&self, id: Option<serde_json::Value>) -> McpResponse {
        McpResponse::success(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "claudebot-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
    }

    /// Handle tools/list
    async fn handle_tools_list(&self, id: Option<serde_json::Value>) -> McpResponse {
        let tools = self.tools.lock().await.list_definitions();
        McpResponse::success(id, serde_json::json!({ "tools": tools }))
    }

    /// Handle tools/call
    async fn handle_tools_call(
        &self,
        id: Option<serde_json::Value>,
        params: serde_json::Value,
    ) -> McpResponse {
        let name = match params.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return McpResponse::error(
                    id,
                    error_codes::INVALID_PARAMS,
                    "Missing 'name' parameter",
                )
            }
        };

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        match self.tools.lock().await.call(name, arguments).await {
            Ok(result) => McpResponse::success(
                id,
                serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": result
                    }]
                }),
            ),
            Err(e) => McpResponse::error(
                id,
                error_codes::TOOL_EXECUTION_ERROR,
                format!("Tool '{}' failed: {}", name, e),
            ),
        }
    }
}
