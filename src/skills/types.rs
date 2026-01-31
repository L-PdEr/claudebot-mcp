//! Skill Type Definitions
//!
//! Core data structures for the skill system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete skill definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    /// Skill metadata
    pub skill: SkillMetadata,
    /// Input parameters
    #[serde(default)]
    pub parameters: HashMap<String, SkillParameter>,
    /// Execution configuration
    pub execution: ExecutionConfig,
    /// Optional examples
    #[serde(default)]
    pub examples: Vec<SkillExample>,
    /// Dependencies on other skills
    #[serde(default)]
    pub dependencies: Vec<String>,
}

impl SkillDefinition {
    /// Create a minimal skill definition
    pub fn new(name: &str, description: &str, execution: ExecutionConfig) -> Self {
        Self {
            skill: SkillMetadata {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: description.to_string(),
                author: None,
                license: Some("MIT".to_string()),
                tags: Vec::new(),
                homepage: None,
            },
            parameters: HashMap::new(),
            execution,
            examples: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// Add a parameter
    pub fn with_parameter(mut self, name: &str, param: SkillParameter) -> Self {
        self.parameters.insert(name.to_string(), param);
        self
    }

    /// Add an example
    pub fn with_example(mut self, example: SkillExample) -> Self {
        self.examples.push(example);
        self
    }

    /// Validate the skill definition
    pub fn validate(&self) -> Result<(), SkillValidationError> {
        // Name validation
        if self.skill.name.is_empty() {
            return Err(SkillValidationError::MissingField("name".to_string()));
        }
        if !self.skill.name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(SkillValidationError::InvalidName(self.skill.name.clone()));
        }

        // Description validation
        if self.skill.description.is_empty() {
            return Err(SkillValidationError::MissingField("description".to_string()));
        }

        // Execution validation
        self.execution.validate()?;

        // Parameter validation
        for (name, param) in &self.parameters {
            if name.is_empty() {
                return Err(SkillValidationError::InvalidParameter(
                    "empty parameter name".to_string(),
                ));
            }
            param.validate()?;
        }

        Ok(())
    }

    /// Convert to JSON Schema for tool registration
    pub fn to_json_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for (name, param) in &self.parameters {
            properties.insert(name.clone(), param.to_json_schema());
            if param.required {
                required.push(serde_json::Value::String(name.clone()));
            }
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }
}

/// Skill metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Unique skill name (alphanumeric + underscore)
    pub name: String,
    /// Semantic version
    pub version: String,
    /// Human-readable description
    pub description: String,
    /// Author name/email
    pub author: Option<String>,
    /// License (default: MIT)
    pub license: Option<String>,
    /// Searchable tags
    #[serde(default)]
    pub tags: Vec<String>,
    /// Homepage URL
    pub homepage: Option<String>,
}

/// Skill parameter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParameter {
    /// Parameter type
    #[serde(rename = "type")]
    pub param_type: ParameterType,
    /// Human-readable description
    pub description: String,
    /// Is this parameter required?
    #[serde(default)]
    pub required: bool,
    /// Default value if not provided
    pub default: Option<serde_json::Value>,
    /// Enum values (for string type)
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    /// Minimum value (for number types)
    pub minimum: Option<f64>,
    /// Maximum value (for number types)
    pub maximum: Option<f64>,
    /// Pattern (for string type)
    pub pattern: Option<String>,
}

impl SkillParameter {
    /// Create a required string parameter
    pub fn string(description: &str, required: bool) -> Self {
        Self {
            param_type: ParameterType::String,
            description: description.to_string(),
            required,
            default: None,
            enum_values: None,
            minimum: None,
            maximum: None,
            pattern: None,
        }
    }

    /// Create a number parameter
    pub fn number(description: &str, required: bool) -> Self {
        Self {
            param_type: ParameterType::Number,
            description: description.to_string(),
            required,
            default: None,
            enum_values: None,
            minimum: None,
            maximum: None,
            pattern: None,
        }
    }

    /// Create a boolean parameter
    pub fn boolean(description: &str, required: bool) -> Self {
        Self {
            param_type: ParameterType::Boolean,
            description: description.to_string(),
            required,
            default: None,
            enum_values: None,
            minimum: None,
            maximum: None,
            pattern: None,
        }
    }

    /// Validate the parameter definition
    pub fn validate(&self) -> Result<(), SkillValidationError> {
        if self.description.is_empty() {
            return Err(SkillValidationError::InvalidParameter(
                "empty description".to_string(),
            ));
        }
        Ok(())
    }

    /// Convert to JSON Schema
    pub fn to_json_schema(&self) -> serde_json::Value {
        let mut schema = serde_json::json!({
            "type": self.param_type.as_str(),
            "description": self.description,
        });

        if let Some(ref default) = self.default {
            schema["default"] = default.clone();
        }
        if let Some(ref enum_values) = self.enum_values {
            schema["enum"] = serde_json::json!(enum_values);
        }
        if let Some(min) = self.minimum {
            schema["minimum"] = serde_json::json!(min);
        }
        if let Some(max) = self.maximum {
            schema["maximum"] = serde_json::json!(max);
        }
        if let Some(ref pattern) = self.pattern {
            schema["pattern"] = serde_json::json!(pattern);
        }

        schema
    }
}

/// Parameter types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
}

impl ParameterType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

/// Execution configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Execution type
    #[serde(rename = "type")]
    pub exec_type: ExecutionType,
    /// HTTP endpoint (for http type)
    pub endpoint: Option<String>,
    /// HTTP method (for http type)
    pub method: Option<String>,
    /// Headers (for http type)
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Shell command template (for shell type)
    pub command: Option<String>,
    /// Script content (for script type)
    pub script: Option<String>,
    /// Script language (for script type)
    pub language: Option<String>,
    /// Claude prompt template (for claude type)
    pub prompt: Option<String>,
    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Retry count
    #[serde(default)]
    pub retries: u32,
}

fn default_timeout() -> u64 {
    30
}

impl ExecutionConfig {
    /// Create HTTP execution config
    pub fn http(endpoint: &str, method: &str) -> Self {
        Self {
            exec_type: ExecutionType::Http,
            endpoint: Some(endpoint.to_string()),
            method: Some(method.to_string()),
            headers: HashMap::new(),
            command: None,
            script: None,
            language: None,
            prompt: None,
            timeout_secs: 30,
            retries: 0,
        }
    }

    /// Create shell execution config
    pub fn shell(command: &str) -> Self {
        Self {
            exec_type: ExecutionType::Shell,
            endpoint: None,
            method: None,
            headers: HashMap::new(),
            command: Some(command.to_string()),
            script: None,
            language: None,
            prompt: None,
            timeout_secs: 30,
            retries: 0,
        }
    }

    /// Create Claude-based execution config
    pub fn claude(prompt: &str) -> Self {
        Self {
            exec_type: ExecutionType::Claude,
            endpoint: None,
            method: None,
            headers: HashMap::new(),
            command: None,
            script: None,
            language: None,
            prompt: Some(prompt.to_string()),
            timeout_secs: 60,
            retries: 0,
        }
    }

    /// Validate execution config
    pub fn validate(&self) -> Result<(), SkillValidationError> {
        match self.exec_type {
            ExecutionType::Http => {
                if self.endpoint.is_none() {
                    return Err(SkillValidationError::MissingField(
                        "execution.endpoint".to_string(),
                    ));
                }
                if self.method.is_none() {
                    return Err(SkillValidationError::MissingField(
                        "execution.method".to_string(),
                    ));
                }
            }
            ExecutionType::Shell => {
                if self.command.is_none() {
                    return Err(SkillValidationError::MissingField(
                        "execution.command".to_string(),
                    ));
                }
            }
            ExecutionType::Script => {
                if self.script.is_none() {
                    return Err(SkillValidationError::MissingField(
                        "execution.script".to_string(),
                    ));
                }
            }
            ExecutionType::Claude => {
                if self.prompt.is_none() {
                    return Err(SkillValidationError::MissingField(
                        "execution.prompt".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Execution types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionType {
    /// HTTP API call
    Http,
    /// Shell command
    Shell,
    /// Embedded script (Python, JS, etc.)
    Script,
    /// Claude-powered (natural language)
    Claude,
}

/// Skill usage example
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillExample {
    /// Example description
    pub description: String,
    /// Example input
    pub input: HashMap<String, serde_json::Value>,
    /// Expected output (for testing)
    pub expected_output: Option<String>,
}

/// Skill validation errors
#[derive(Debug, thiserror::Error)]
pub enum SkillValidationError {
    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid skill name: {0}")]
    InvalidName(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Invalid execution config: {0}")]
    InvalidExecution(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_definition_basic() {
        let skill = SkillDefinition::new(
            "weather",
            "Get current weather",
            ExecutionConfig::http("https://api.weather.com", "GET"),
        )
        .with_parameter("location", SkillParameter::string("City name", true));

        assert!(skill.validate().is_ok());
        assert_eq!(skill.skill.name, "weather");
    }

    #[test]
    fn test_skill_validation_missing_name() {
        let skill = SkillDefinition::new(
            "",
            "Description",
            ExecutionConfig::shell("echo hello"),
        );

        assert!(skill.validate().is_err());
    }

    #[test]
    fn test_parameter_to_json_schema() {
        let param = SkillParameter::string("A test parameter", true);
        let schema = param.to_json_schema();

        assert_eq!(schema["type"], "string");
        assert_eq!(schema["description"], "A test parameter");
    }

    #[test]
    fn test_skill_to_json_schema() {
        let skill = SkillDefinition::new(
            "test",
            "Test skill",
            ExecutionConfig::shell("echo {{input}}"),
        )
        .with_parameter("input", SkillParameter::string("Input text", true));

        let schema = skill.to_json_schema();

        assert!(schema["properties"]["input"].is_object());
        assert_eq!(schema["required"][0], "input");
    }
}
