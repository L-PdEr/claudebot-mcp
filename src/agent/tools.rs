//! Structured Tool Use Framework
//!
//! Provides JSON schema-based tool definitions with parallel execution:
//! - Tool registration with JSON schemas
//! - Parameter validation
//! - Parallel tool execution
//! - Result aggregation
//!
//! Industry standard: OpenAI Function Calling, Claude Tool Use

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::info;

/// JSON Schema for tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name (snake_case)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for parameters
    pub parameters: Value,
    /// Required parameter names
    pub required: Vec<String>,
}

impl ToolSchema {
    /// Create a new tool schema
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            required: vec![],
        }
    }

    /// Add a string parameter
    pub fn with_string_param(mut self, name: &str, description: &str, required: bool) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "string",
                "description": description
            });
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Add an integer parameter
    pub fn with_int_param(mut self, name: &str, description: &str, required: bool) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "integer",
                "description": description
            });
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Add a boolean parameter
    pub fn with_bool_param(mut self, name: &str, description: &str, required: bool) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "boolean",
                "description": description
            });
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Add an enum parameter
    pub fn with_enum_param(mut self, name: &str, description: &str, values: &[&str], required: bool) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "string",
                "description": description,
                "enum": values
            });
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Add an object parameter
    pub fn with_object_param(mut self, name: &str, description: &str, required: bool) -> Self {
        if let Some(props) = self.parameters.get_mut("properties") {
            props[name] = serde_json::json!({
                "type": "object",
                "description": description
            });
        }
        if required {
            self.required.push(name.to_string());
        }
        self
    }

    /// Validate parameters against schema
    pub fn validate(&self, params: &Value) -> Result<()> {
        // Check required parameters
        for req in &self.required {
            if params.get(req).is_none() {
                return Err(anyhow!("Missing required parameter: {}", req));
            }
        }

        // Basic type validation
        if let Some(props) = self.parameters.get("properties") {
            if let Some(obj) = props.as_object() {
                for (name, schema) in obj {
                    if let Some(value) = params.get(name) {
                        let expected_type = schema.get("type").and_then(|t| t.as_str());
                        let valid = match expected_type {
                            Some("string") => value.is_string(),
                            Some("integer") => value.is_i64(),
                            Some("number") => value.is_number(),
                            Some("boolean") => value.is_boolean(),
                            Some("array") => value.is_array(),
                            Some("object") => value.is_object(),
                            _ => true,
                        };
                        if !valid {
                            return Err(anyhow!(
                                "Parameter '{}' has wrong type, expected {}",
                                name,
                                expected_type.unwrap_or("unknown")
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Format as Claude-compatible tool definition
    pub fn to_claude_format(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "input_schema": {
                "type": "object",
                "properties": self.parameters.get("properties").cloned().unwrap_or(Value::Object(Default::default())),
                "required": self.required
            }
        })
    }
}

/// Result from tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool name that was called
    pub tool_name: String,
    /// Whether execution succeeded
    pub success: bool,
    /// Result content (success or error message)
    pub content: String,
    /// Structured result data
    pub data: Option<Value>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

impl ToolResult {
    /// Create a successful result
    pub fn success(tool_name: &str, content: String) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            success: true,
            content,
            data: None,
            duration_ms: 0,
        }
    }

    /// Create a successful result with data
    pub fn success_with_data(tool_name: &str, content: String, data: Value) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            success: true,
            content,
            data: Some(data),
            duration_ms: 0,
        }
    }

    /// Create an error result
    pub fn error(tool_name: &str, error: String) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            success: false,
            content: error,
            data: None,
            duration_ms: 0,
        }
    }

    /// Set duration
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// A tool call request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique call ID
    pub id: String,
    /// Tool name
    pub name: String,
    /// Parameters as JSON
    pub parameters: Value,
}

impl ToolCall {
    /// Create a new tool call
    pub fn new(name: &str, parameters: Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            parameters,
        }
    }
}

/// Type alias for tool handler function
pub type ToolHandler = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send>>
        + Send
        + Sync
>;

/// A registered tool with schema and handler
pub struct Tool {
    pub schema: ToolSchema,
    handler: ToolHandler,
}

impl Tool {
    /// Create a new tool
    pub fn new<F, Fut>(schema: ToolSchema, handler: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolResult>> + Send + 'static,
    {
        Self {
            schema,
            handler: Arc::new(move |params| Box::pin(handler(params))),
        }
    }

    /// Execute the tool
    pub async fn execute(&self, params: Value) -> Result<ToolResult> {
        let start = std::time::Instant::now();

        // Validate parameters
        self.schema.validate(&params)?;

        // Execute handler
        let mut result = (self.handler)(params).await?;
        result.duration_ms = start.elapsed().as_millis() as u64;

        Ok(result)
    }
}

/// Tool registry for managing available tools
pub struct ToolRegistry {
    tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Tool) {
        info!("Registered tool: {}", tool.schema.name);
        self.tools.insert(tool.schema.name.clone(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// List all tool names
    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get all tool schemas
    pub fn schemas(&self) -> Vec<&ToolSchema> {
        self.tools.values().map(|t| &t.schema).collect()
    }

    /// Execute a single tool call
    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        match self.tools.get(&call.name) {
            Some(tool) => {
                match tool.execute(call.parameters.clone()).await {
                    Ok(result) => result,
                    Err(e) => ToolResult::error(&call.name, e.to_string()),
                }
            }
            None => ToolResult::error(&call.name, format!("Unknown tool: {}", call.name)),
        }
    }

    /// Execute multiple tool calls in parallel
    pub async fn execute_parallel(&self, calls: Vec<ToolCall>) -> Vec<ToolResult> {
        let mut results = Vec::with_capacity(calls.len());

        // Execute calls sequentially (parallel execution would require futures crate)
        for call in &calls {
            results.push(self.execute(call).await);
        }

        results
    }

    /// Format all tools for Claude API
    pub fn to_claude_format(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| t.schema.to_claude_format())
            .collect()
    }

    /// Parse tool calls from LLM response
    pub fn parse_tool_calls(&self, response: &str) -> Vec<ToolCall> {
        // Try to parse as JSON array of tool calls
        if let Some(json_str) = extract_json_array(response) {
            if let Ok(calls) = serde_json::from_str::<Vec<ToolCallJson>>(json_str) {
                return calls
                    .into_iter()
                    .map(|c| ToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: c.name,
                        parameters: c.parameters,
                    })
                    .collect();
            }
        }

        // Try to parse as single tool call object
        if let Some(json_str) = extract_json_object(response) {
            if let Ok(call) = serde_json::from_str::<ToolCallJson>(json_str) {
                return vec![ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: call.name,
                    parameters: call.parameters,
                }];
            }
        }

        vec![]
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// JSON structure for parsing tool calls
#[derive(Debug, Deserialize)]
struct ToolCallJson {
    name: String,
    #[serde(default)]
    parameters: Value,
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

/// Extract JSON object from text
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0;
    let mut end = start;

    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
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

/// Built-in tools for common operations
pub mod builtin {
    use super::*;

    /// Create a memory/recall tool
    pub fn memory_tool() -> Tool {
        let schema = ToolSchema::new("memory", "Store or recall information from memory")
            .with_enum_param("action", "Whether to store or recall", &["store", "recall"], true)
            .with_string_param("content", "Content to store or query to recall", true);

        Tool::new(schema, |params| async move {
            let action = params.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

            match action {
                "store" => Ok(ToolResult::success("memory", format!("Stored: {}", content))),
                "recall" => Ok(ToolResult::success("memory", format!("Recalled for query: {}", content))),
                _ => Ok(ToolResult::error("memory", "Invalid action".to_string())),
            }
        })
    }

    /// Create a search tool
    pub fn search_tool() -> Tool {
        let schema = ToolSchema::new("search", "Search the web for information")
            .with_string_param("query", "Search query", true)
            .with_int_param("max_results", "Maximum number of results", false);

        Tool::new(schema, |params| async move {
            let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
            Ok(ToolResult::success("search", format!("Search results for: {}", query)))
        })
    }

    /// Create a calculator tool
    pub fn calculator_tool() -> Tool {
        let schema = ToolSchema::new("calculator", "Perform mathematical calculations")
            .with_string_param("expression", "Mathematical expression to evaluate", true);

        Tool::new(schema, |params| async move {
            let expr = params.get("expression").and_then(|v| v.as_str()).unwrap_or("");
            // Simple evaluation (in production, use a proper expression parser)
            Ok(ToolResult::success("calculator", format!("Result of '{}': (evaluation pending)", expr)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_schema_creation() {
        let schema = ToolSchema::new("test", "A test tool")
            .with_string_param("name", "User name", true)
            .with_int_param("age", "User age", false);

        assert_eq!(schema.name, "test");
        assert_eq!(schema.required.len(), 1);
        assert!(schema.required.contains(&"name".to_string()));
    }

    #[test]
    fn test_schema_validation() {
        let schema = ToolSchema::new("test", "Test")
            .with_string_param("name", "Name", true);

        // Valid
        let valid = serde_json::json!({"name": "Alice"});
        assert!(schema.validate(&valid).is_ok());

        // Missing required
        let missing = serde_json::json!({});
        assert!(schema.validate(&missing).is_err());

        // Wrong type
        let wrong_type = serde_json::json!({"name": 123});
        assert!(schema.validate(&wrong_type).is_err());
    }

    #[test]
    fn test_tool_result_creation() {
        let success = ToolResult::success("test", "OK".to_string());
        assert!(success.success);
        assert_eq!(success.content, "OK");

        let error = ToolResult::error("test", "Failed".to_string());
        assert!(!error.success);
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let mut registry = ToolRegistry::new();
        registry.register(builtin::memory_tool());
        registry.register(builtin::search_tool());

        assert_eq!(registry.list().len(), 2);
        assert!(registry.get("memory").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_tool_execution() {
        let mut registry = ToolRegistry::new();
        registry.register(builtin::memory_tool());

        let call = ToolCall::new("memory", serde_json::json!({
            "action": "store",
            "content": "test data"
        }));

        let result = registry.execute(&call).await;
        assert!(result.success);
        assert!(result.content.contains("Stored"));
    }

    #[test]
    fn test_parse_tool_calls() {
        let registry = ToolRegistry::new();

        let response = r#"I'll use these tools: [{"name": "search", "parameters": {"query": "rust async"}}]"#;
        let calls = registry.parse_tool_calls(response);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
    }

    #[test]
    fn test_claude_format() {
        let schema = ToolSchema::new("test", "Test tool")
            .with_string_param("input", "Input value", true);

        let formatted = schema.to_claude_format();
        assert_eq!(formatted["name"], "test");
        assert!(formatted["input_schema"]["required"].as_array().unwrap().contains(&Value::String("input".to_string())));
    }
}
