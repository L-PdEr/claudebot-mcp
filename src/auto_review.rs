//! Llama-Powered Autonomous Code Review
//!
//! Uses local Llama model to:
//! - Assess risk of code changes
//! - Decide if auto-approval is safe
//! - Generate commit messages
//! - Pre-review before human review
//!
//! This is how elite teams automate quality:
//! 1. Static analysis (lint, types)
//! 2. AI pre-review (catch obvious issues)
//! 3. Automated tests
//! 4. Human review (only for complex changes)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::llama_worker::LlamaWorker;
use crate::permissions::{RiskAssessment, RiskCategory, ApprovalRecommendation, ChangeProposal};

/// Autonomous reviewer using Llama
pub struct AutoReviewer {
    llama: LlamaWorker,
    config: ReviewConfig,
}

/// Configuration for auto-review
#[derive(Debug, Clone)]
pub struct ReviewConfig {
    /// Maximum lines for auto-approval
    pub max_auto_approve_lines: usize,
    /// Files that always need human review
    pub sensitive_patterns: Vec<String>,
    /// Enable auto-commit for trivial changes
    pub enable_auto_commit: bool,
    /// Enable auto-push (requires autonomous mode)
    pub enable_auto_push: bool,
    /// Run tests before approval
    pub require_tests: bool,
    /// Run lint before approval
    pub require_lint: bool,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            max_auto_approve_lines: 50,
            sensitive_patterns: vec![
                "**/auth/**".to_string(),
                "**/payment*".to_string(),
                "**/secret*".to_string(),
                "**/.env*".to_string(),
                "**/wallet*".to_string(),
                "**/key*".to_string(),
            ],
            enable_auto_commit: true,
            enable_auto_push: false,
            require_tests: true,
            require_lint: true,
        }
    }
}

/// Diff information for review
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffInfo {
    pub files: Vec<FileDiff>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub content: String,
}

/// Review result from Llama
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub risk: RiskAssessment,
    pub summary: String,
    pub issues: Vec<ReviewIssue>,
    pub suggested_commit_message: String,
    pub can_auto_approve: bool,
    pub requires_tests: bool,
    pub requires_human: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    pub severity: IssueSeverity,
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

impl AutoReviewer {
    pub fn new(config: ReviewConfig) -> Self {
        Self {
            llama: LlamaWorker::new(),
            config,
        }
    }

    /// Check if Llama is available for review
    pub async fn is_available(&self) -> bool {
        self.llama.is_available().await
    }

    /// Review a diff and provide assessment
    pub async fn review_diff(&self, diff: &DiffInfo, context: &str) -> Result<ReviewResult> {
        if !self.is_available().await {
            return Ok(self.fallback_review(diff));
        }

        let prompt = self.build_review_prompt(diff, context);
        let response = self.llama.generate(&prompt).await?;

        self.parse_review_response(&response, diff)
    }

    /// Assess risk of changes
    pub async fn assess_risk(&self, diff: &DiffInfo) -> RiskAssessment {
        let mut score: u8 = 0;
        let mut concerns = Vec::new();

        // Size-based risk
        let total_changes = diff.total_additions + diff.total_deletions;
        if total_changes > 500 {
            score += 3;
            concerns.push("Large changeset (>500 lines)".to_string());
        } else if total_changes > 100 {
            score += 1;
        }

        // Sensitive file detection
        for file in &diff.files {
            for pattern in &self.config.sensitive_patterns {
                if glob_match(pattern, &file.path) {
                    score += 4;
                    concerns.push(format!("Sensitive file: {}", file.path));
                    break;
                }
            }

            // Check for security-related content
            let content_lower = file.content.to_lowercase();
            if content_lower.contains("password") ||
               content_lower.contains("secret") ||
               content_lower.contains("api_key") ||
               content_lower.contains("token") {
                score += 3;
                concerns.push(format!("Security-sensitive content in {}", file.path));
            }

            // Check for dangerous operations
            if content_lower.contains("drop table") ||
               content_lower.contains("delete from") ||
               content_lower.contains("rm -rf") ||
               content_lower.contains("format!") && content_lower.contains("unsafe") {
                score += 5;
                concerns.push(format!("Potentially dangerous operation in {}", file.path));
            }
        }

        // Determine category and recommendation
        let (category, recommendation) = match score {
            0..=2 => (RiskCategory::Trivial, ApprovalRecommendation::AutoApprove),
            3..=4 => (RiskCategory::Low, ApprovalRecommendation::QuickReview),
            5..=6 => (RiskCategory::Medium, ApprovalRecommendation::FullReview),
            7..=8 => (RiskCategory::High, ApprovalRecommendation::ExpertReview),
            _ => (RiskCategory::Critical, ApprovalRecommendation::Block),
        };

        RiskAssessment {
            score: score.min(10),
            category,
            concerns,
            recommendation,
        }
    }

    /// Generate commit message for changes
    pub async fn generate_commit_message(&self, diff: &DiffInfo, description: &str) -> Result<String> {
        if !self.is_available().await {
            return Ok(self.fallback_commit_message(diff, description));
        }

        let prompt = format!(
            r#"Generate a conventional commit message for these changes.

Description: {}

Files changed:
{}

Format: <type>(<scope>): <description>

Types: feat, fix, refactor, docs, test, chore, perf, style
Keep it under 72 characters.
Only output the commit message, nothing else."#,
            description,
            diff.files.iter()
                .map(|f| format!("- {} (+{}, -{})", f.path, f.additions, f.deletions))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let response = self.llama.generate(&prompt).await?;
        Ok(response.lines().next().unwrap_or("chore: update code").trim().to_string())
    }

    /// Check if changes can be auto-approved
    pub fn can_auto_approve(&self, diff: &DiffInfo, risk: &RiskAssessment) -> bool {
        // Check size threshold
        let total = diff.total_additions + diff.total_deletions;
        if total > self.config.max_auto_approve_lines {
            return false;
        }

        // Check risk level
        matches!(
            risk.recommendation,
            ApprovalRecommendation::AutoApprove
        )
    }

    /// Build the review prompt for Llama
    fn build_review_prompt(&self, diff: &DiffInfo, context: &str) -> String {
        let files_summary = diff.files.iter()
            .map(|f| format!("## {}\n+{} -{}\n```\n{}\n```",
                f.path, f.additions, f.deletions,
                truncate(&f.content, 1000)))
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            r#"You are a senior code reviewer. Review this diff and provide:
1. Risk assessment (0-10)
2. Issues found (with severity: info/warning/error/critical)
3. Whether it can be auto-approved
4. Suggested commit message

Context: {}

## Changes
{}

Respond in JSON format:
{{
  "risk_score": <0-10>,
  "summary": "<brief summary>",
  "issues": [
    {{"severity": "warning", "file": "path", "line": 42, "message": "issue", "suggestion": "fix"}}
  ],
  "commit_message": "<conventional commit>",
  "can_auto_approve": <true/false>,
  "requires_tests": <true/false>,
  "requires_human": <true/false>
}}"#,
            context,
            files_summary
        )
    }

    /// Parse Llama response into ReviewResult
    fn parse_review_response(&self, response: &str, diff: &DiffInfo) -> Result<ReviewResult> {
        // Try to extract JSON from response
        let json_start = response.find('{');
        let json_end = response.rfind('}');

        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_str = &response[start..=end];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let risk_score = parsed["risk_score"].as_u64().unwrap_or(5) as u8;
                let can_auto = parsed["can_auto_approve"].as_bool().unwrap_or(false);

                let issues: Vec<ReviewIssue> = parsed["issues"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                Some(ReviewIssue {
                                    severity: match v["severity"].as_str()? {
                                        "info" => IssueSeverity::Info,
                                        "warning" => IssueSeverity::Warning,
                                        "error" => IssueSeverity::Error,
                                        "critical" => IssueSeverity::Critical,
                                        _ => IssueSeverity::Info,
                                    },
                                    file: v["file"].as_str()?.to_string(),
                                    line: v["line"].as_u64().map(|n| n as usize),
                                    message: v["message"].as_str()?.to_string(),
                                    suggestion: v["suggestion"].as_str().map(|s| s.to_string()),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let risk = RiskAssessment {
                    score: risk_score,
                    category: match risk_score {
                        0..=2 => RiskCategory::Trivial,
                        3..=4 => RiskCategory::Low,
                        5..=6 => RiskCategory::Medium,
                        7..=8 => RiskCategory::High,
                        _ => RiskCategory::Critical,
                    },
                    concerns: issues.iter()
                        .filter(|i| i.severity == IssueSeverity::Error || i.severity == IssueSeverity::Critical)
                        .map(|i| i.message.clone())
                        .collect(),
                    recommendation: if can_auto {
                        ApprovalRecommendation::AutoApprove
                    } else if risk_score <= 4 {
                        ApprovalRecommendation::QuickReview
                    } else {
                        ApprovalRecommendation::FullReview
                    },
                };

                return Ok(ReviewResult {
                    risk,
                    summary: parsed["summary"].as_str().unwrap_or("").to_string(),
                    issues,
                    suggested_commit_message: parsed["commit_message"]
                        .as_str()
                        .unwrap_or("chore: update code")
                        .to_string(),
                    can_auto_approve: can_auto,
                    requires_tests: parsed["requires_tests"].as_bool().unwrap_or(true),
                    requires_human: parsed["requires_human"].as_bool().unwrap_or(true),
                });
            }
        }

        // Fallback if parsing fails
        Ok(self.fallback_review(diff))
    }

    /// Fallback review when Llama is unavailable
    fn fallback_review(&self, diff: &DiffInfo) -> ReviewResult {
        let total = diff.total_additions + diff.total_deletions;
        let is_small = total <= self.config.max_auto_approve_lines;

        let category = if total <= 20 {
            RiskCategory::Trivial
        } else if total <= 50 {
            RiskCategory::Low
        } else if total <= 200 {
            RiskCategory::Medium
        } else {
            RiskCategory::High
        };

        ReviewResult {
            risk: RiskAssessment {
                score: if is_small { 2 } else { 5 },
                category,
                concerns: vec![],
                recommendation: if is_small {
                    ApprovalRecommendation::QuickReview
                } else {
                    ApprovalRecommendation::FullReview
                },
            },
            summary: format!("Changed {} files ({} additions, {} deletions)",
                diff.files.len(), diff.total_additions, diff.total_deletions),
            issues: vec![],
            suggested_commit_message: self.fallback_commit_message(diff, ""),
            can_auto_approve: is_small && !self.has_sensitive_files(diff),
            requires_tests: total > 10,
            requires_human: !is_small,
        }
    }

    /// Generate fallback commit message
    fn fallback_commit_message(&self, diff: &DiffInfo, description: &str) -> String {
        if !description.is_empty() {
            return format!("chore: {}", truncate(description, 60));
        }

        let file_types: Vec<&str> = diff.files.iter()
            .filter_map(|f| f.path.split('.').last())
            .collect();

        let scope = if file_types.contains(&"rs") {
            "rust"
        } else if file_types.contains(&"ts") || file_types.contains(&"vue") {
            "frontend"
        } else if file_types.contains(&"md") {
            "docs"
        } else {
            "misc"
        };

        let action = if diff.total_additions > diff.total_deletions * 2 {
            "add"
        } else if diff.total_deletions > diff.total_additions * 2 {
            "remove"
        } else {
            "update"
        };

        format!("chore({}): {} code", scope, action)
    }

    /// Check if diff contains sensitive files
    fn has_sensitive_files(&self, diff: &DiffInfo) -> bool {
        diff.files.iter().any(|f| {
            self.config.sensitive_patterns.iter()
                .any(|pattern| glob_match(pattern, &f.path))
        })
    }
}

/// Simple glob matching
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.contains("**") {
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');
            return (prefix.is_empty() || path.starts_with(prefix)) &&
                   (suffix.is_empty() || path.ends_with(suffix) || path.contains(&format!("/{}", suffix)));
        }
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;
        for part in parts {
            if part.is_empty() { continue; }
            if let Some(idx) = path[pos..].find(part) {
                pos += idx + part.len();
            } else {
                return false;
            }
        }
        return true;
    }
    path == pattern
}

/// Truncate string with ellipsis
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_assessment() {
        let reviewer = AutoReviewer::new(ReviewConfig::default());

        let small_diff = DiffInfo {
            files: vec![FileDiff {
                path: "src/lib.rs".to_string(),
                additions: 5,
                deletions: 2,
                content: "fn foo() {}".to_string(),
            }],
            total_additions: 5,
            total_deletions: 2,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let risk = rt.block_on(reviewer.assess_risk(&small_diff));
        assert!(risk.score <= 2);
        assert_eq!(risk.category, RiskCategory::Trivial);
    }

    #[test]
    fn test_sensitive_detection() {
        let reviewer = AutoReviewer::new(ReviewConfig::default());

        let sensitive_diff = DiffInfo {
            files: vec![FileDiff {
                path: "src/auth/login.rs".to_string(),
                additions: 10,
                deletions: 5,
                content: "password validation".to_string(),
            }],
            total_additions: 10,
            total_deletions: 5,
        };

        assert!(reviewer.has_sensitive_files(&sensitive_diff));
    }
}
