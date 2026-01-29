//! Bridge Types
//!
//! Internal types for bridge communication.

use serde::Deserialize;

/// Internal execution result from Claude CLI JSON output
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeCliOutput {
    #[serde(rename = "type")]
    pub output_type: String,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub duration_api_ms: Option<u64>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_cli_output_parsing() {
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "cost_usd": 0.01,
            "duration_ms": 2000,
            "session_id": "session-xyz",
            "result": "Task completed successfully"
        }"#;

        let output: ClaudeCliOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.output_type, "result");
        assert_eq!(output.session_id, Some("session-xyz".to_string()));
        assert_eq!(output.result, Some("Task completed successfully".to_string()));
    }

    #[test]
    fn test_claude_cli_assistant_message() {
        let json = r#"{
            "type": "assistant",
            "message": "I'll help you with that."
        }"#;

        let output: ClaudeCliOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.output_type, "assistant");
        assert_eq!(output.message, Some("I'll help you with that.".to_string()));
    }

    #[test]
    fn test_claude_cli_system_message() {
        let json = r#"{
            "type": "system",
            "session_id": "abc-123"
        }"#;

        let output: ClaudeCliOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.output_type, "system");
        assert_eq!(output.session_id, Some("abc-123".to_string()));
    }
}
