//! Skill Generator
//!
//! Uses Claude to generate new skills from natural language descriptions.
//! This is the core of the self-extending capability.

use super::types::*;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Generated skill with metadata
#[derive(Debug, Clone)]
pub struct GeneratedSkill {
    /// The skill definition
    pub definition: SkillDefinition,
    /// Generation confidence (0.0 - 1.0)
    pub confidence: f64,
    /// Reasoning for the generated skill
    pub reasoning: String,
    /// Suggested tests
    pub tests: Vec<SkillTest>,
    /// Whether this needs user approval
    pub needs_approval: bool,
}

/// Test case for a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTest {
    pub name: String,
    pub input: serde_json::Value,
    pub expected_contains: Option<String>,
    pub expected_success: bool,
}

/// Skill generator using LLM
pub struct SkillGenerator {
    /// HTTP client for Claude API
    client: reqwest::Client,
    /// Anthropic API key
    api_key: String,
    /// Model to use for generation
    model: String,
    /// Maximum retries
    max_retries: u32,
}

impl SkillGenerator {
    /// Create new generator from environment
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set")?;

        Ok(Self {
            client: reqwest::Client::new(),
            api_key,
            model: "claude-sonnet-4-20250514".to_string(),
            max_retries: 2,
        })
    }

    /// Create with custom config
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_retries: 2,
        }
    }

    /// Detect if a message implies a need for a new skill
    pub fn detect_skill_need(&self, message: &str) -> Option<SkillNeed> {
        let lower = message.to_lowercase();

        // Explicit skill requests
        let explicit_patterns = [
            "create a skill",
            "add a tool",
            "make a tool",
            "i need a skill",
            "can you create",
            "automate",
            "whenever i say",
            "every time i",
        ];

        for pattern in explicit_patterns {
            if lower.contains(pattern) {
                return Some(SkillNeed {
                    description: message.to_string(),
                    is_explicit: true,
                    priority: SkillPriority::High,
                });
            }
        }

        // Implicit skill needs (repeated requests)
        let implicit_patterns = [
            ("fetch", "http"),
            ("get weather", "weather"),
            ("translate", "translation"),
            ("search", "search"),
            ("calculate", "calculator"),
            ("convert", "converter"),
            ("remind me", "reminder"),
            ("schedule", "scheduler"),
            ("send email", "email"),
            ("post to", "social"),
        ];

        for (pattern, skill_type) in implicit_patterns {
            if lower.contains(pattern) {
                return Some(SkillNeed {
                    description: format!("{} skill for: {}", skill_type, message),
                    is_explicit: false,
                    priority: SkillPriority::Medium,
                });
            }
        }

        None
    }

    /// Generate a skill from natural language description
    pub async fn generate(&self, description: &str) -> Result<GeneratedSkill> {
        let prompt = format!(
            r#"Generate a skill definition in TOML format based on this description:

{description}

The skill should be:
1. Self-contained and reusable
2. Have clear parameter definitions
3. Include appropriate execution type (http, shell, script, or claude)
4. Be safe to execute (no destructive operations without confirmation)

Return a JSON object with this structure:
{{
  "skill": {{
    "name": "skill_name",
    "version": "1.0.0",
    "description": "What the skill does"
  }},
  "parameters": {{
    "param_name": {{
      "type": "string",
      "description": "Parameter description",
      "required": true
    }}
  }},
  "execution": {{
    "type": "http|shell|script|claude",
    // For http:
    "endpoint": "https://...",
    "method": "GET|POST",
    // For shell:
    "command": "command with {{{{param_name}}}}",
    // For script:
    "script": "code here",
    "language": "python|javascript",
    // For claude:
    "prompt": "prompt template with {{{{param_name}}}}"
  }},
  "confidence": 0.0-1.0,
  "reasoning": "Why this skill design was chosen",
  "tests": [
    {{
      "name": "test_name",
      "input": {{"param": "value"}},
      "expected_success": true
    }}
  ]
}}

Return ONLY valid JSON, no markdown or explanation."#
        );

        let response = self.call_claude(&prompt).await?;
        self.parse_generated_skill(&response, description)
    }

    /// Generate a skill from existing tool usage patterns
    pub async fn generate_from_patterns(
        &self,
        patterns: &[ToolUsagePattern],
    ) -> Result<GeneratedSkill> {
        if patterns.is_empty() {
            anyhow::bail!("No patterns provided");
        }

        let patterns_json = serde_json::to_string_pretty(patterns)?;

        let prompt = format!(
            r#"Analyze these tool usage patterns and create a reusable skill:

{patterns_json}

The skill should:
1. Capture the common pattern across these usages
2. Abstract variable parts as parameters
3. Be more efficient than repeated manual operations

Return a JSON skill definition (same format as skill generation)."#
        );

        let response = self.call_claude(&prompt).await?;
        self.parse_generated_skill(&response, "pattern-based generation")
    }

    /// Call Claude API
    async fn call_claude(&self, prompt: &str) -> Result<String> {
        let mut last_error = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500 * attempt as u64)).await;
            }

            let response = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "model": self.model,
                    "max_tokens": 4096,
                    "messages": [
                        {"role": "user", "content": prompt}
                    ]
                }))
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await?;
                    if let Some(text) = body["content"][0]["text"].as_str() {
                        return Ok(text.to_string());
                    }
                    anyhow::bail!("Unexpected response format");
                }
                Ok(resp) => {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    last_error = Some(anyhow::anyhow!("API error {}: {}", status, error_text));
                }
                Err(e) => {
                    last_error = Some(e.into());
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error")))
    }

    /// Parse Claude's response into a GeneratedSkill
    fn parse_generated_skill(&self, response: &str, description: &str) -> Result<GeneratedSkill> {
        // Extract JSON from response (may be wrapped in markdown)
        let json_str = extract_json(response).unwrap_or(response);

        #[derive(Deserialize)]
        struct GeneratedResponse {
            skill: SkillMetadata,
            #[serde(default)]
            parameters: std::collections::HashMap<String, SkillParameter>,
            execution: ExecutionConfig,
            #[serde(default = "default_confidence")]
            confidence: f64,
            #[serde(default)]
            reasoning: String,
            #[serde(default)]
            tests: Vec<SkillTest>,
        }

        fn default_confidence() -> f64 {
            0.7
        }

        let parsed: GeneratedResponse = serde_json::from_str(json_str)
            .context("Failed to parse generated skill JSON")?;

        let definition = SkillDefinition {
            skill: parsed.skill,
            parameters: parsed.parameters,
            execution: parsed.execution,
            examples: Vec::new(),
            dependencies: Vec::new(),
        };

        // Validate the definition
        definition.validate()
            .context("Generated skill failed validation")?;

        // Determine if approval is needed
        let needs_approval = matches!(
            definition.execution.exec_type,
            ExecutionType::Shell | ExecutionType::Script
        ) || parsed.confidence < 0.8;

        info!(
            "Generated skill '{}' with confidence {:.0}%",
            definition.skill.name,
            parsed.confidence * 100.0
        );

        Ok(GeneratedSkill {
            definition,
            confidence: parsed.confidence,
            reasoning: parsed.reasoning,
            tests: parsed.tests,
            needs_approval,
        })
    }

    /// Improve an existing skill based on feedback
    pub async fn improve(&self, skill: &SkillDefinition, feedback: &str) -> Result<GeneratedSkill> {
        let skill_json = serde_json::to_string_pretty(skill)?;

        let prompt = format!(
            r#"Improve this skill based on the feedback:

Current skill:
{skill_json}

Feedback:
{feedback}

Return an improved JSON skill definition."#
        );

        let response = self.call_claude(&prompt).await?;
        self.parse_generated_skill(&response, "skill improvement")
    }
}

/// Detected skill need
#[derive(Debug, Clone)]
pub struct SkillNeed {
    pub description: String,
    pub is_explicit: bool,
    pub priority: SkillPriority,
}

/// Skill priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPriority {
    Low,
    Medium,
    High,
}

/// Tool usage pattern for learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsagePattern {
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub context: String,
    pub frequency: u32,
    pub success_rate: f64,
}

/// Extract JSON from a string that may contain markdown
fn extract_json(s: &str) -> Option<&str> {
    // Try to find JSON object
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_skill_need_explicit() {
        let gen = SkillGenerator::new("test", "test");

        let need = gen.detect_skill_need("can you create a skill to check stock prices");
        assert!(need.is_some());
        assert!(need.unwrap().is_explicit);
    }

    #[test]
    fn test_detect_skill_need_implicit() {
        let gen = SkillGenerator::new("test", "test");

        let need = gen.detect_skill_need("get weather in Berlin");
        assert!(need.is_some());
        assert!(!need.unwrap().is_explicit);
    }

    #[test]
    fn test_detect_skill_need_none() {
        let gen = SkillGenerator::new("test", "test");

        let need = gen.detect_skill_need("hello, how are you?");
        assert!(need.is_none());
    }

    #[test]
    fn test_extract_json() {
        let text = "Here is the skill:\n```json\n{\"name\": \"test\"}\n```";
        let json = extract_json(text);
        assert_eq!(json, Some("{\"name\": \"test\"}"));
    }
}
