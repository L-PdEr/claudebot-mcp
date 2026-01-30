//! Task Feedback System
//!
//! Provides structured feedback for task completion.
//! Parses Claude Code output to extract actions, files, commits, etc.

use once_cell::sync::Lazy;
use regex::Regex;
use std::time::Duration;

/// Summary of a completed task
#[derive(Debug, Default)]
pub struct TaskSummary {
    pub success: bool,
    pub duration: Duration,
    pub actions: Vec<TaskAction>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub code_diff: Option<String>,
    pub exit_code: Option<i32>,
}

/// Individual actions taken during a task
#[derive(Debug, Clone)]
pub enum TaskAction {
    FileCreated { path: String, lines: usize },
    FileModified { path: String, added: usize, removed: usize },
    FileDeleted { path: String },
    GitCommit { hash: String, message: String },
    GitBranch { name: String, action: BranchAction },
    GitPush { branch: String, remote: String },
    TestsRan { passed: usize, failed: usize, skipped: usize },
    BuildCompleted { artifact: String, size_bytes: Option<u64> },
    CommandRan { cmd: String, exit_code: i32 },
}

#[derive(Debug, Clone)]
pub enum BranchAction {
    Created,
    Switched,
    Merged,
    Deleted,
}

impl BranchAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Switched => "switched to",
            Self::Merged => "merged",
            Self::Deleted => "deleted",
        }
    }
}

/// Feedback formatter for Telegram
pub struct TaskFeedback;

impl TaskFeedback {
    /// Format task summary for Telegram (HTML)
    pub fn format_telegram(summary: &TaskSummary) -> String {
        let mut msg = String::new();

        // Status header
        if summary.errors.is_empty() && summary.success {
            msg.push_str("<b>Task completed successfully</b>\n\n");
        } else if !summary.errors.is_empty() {
            msg.push_str("<b>Task failed</b>\n\n");
        } else {
            msg.push_str("<b>Task completed with warnings</b>\n\n");
        }

        // Actions summary
        if !summary.actions.is_empty() {
            msg.push_str("<b>Summary:</b>\n");
            for action in &summary.actions {
                msg.push_str(&format!("  {}\n", Self::format_action(action)));
            }
            msg.push('\n');
        }

        // Code diff preview
        if let Some(ref diff) = summary.code_diff {
            if !diff.is_empty() {
                msg.push_str("<b>Changes:</b>\n");
                let preview = Self::truncate_diff(diff, 500);
                msg.push_str(&format!("<pre>{}</pre>\n\n", Self::html_escape(&preview)));
            }
        }

        // Warnings
        if !summary.warnings.is_empty() {
            msg.push_str("<b>Warnings:</b>\n");
            for warn in &summary.warnings {
                msg.push_str(&format!("  {}\n", Self::html_escape(warn)));
            }
            msg.push('\n');
        }

        // Errors
        if !summary.errors.is_empty() {
            msg.push_str("<b>Errors:</b>\n");
            for err in &summary.errors {
                msg.push_str(&format!("  {}\n", Self::html_escape(err)));
            }
            msg.push('\n');
        }

        // Duration
        msg.push_str(&format!("Duration: {}", Self::format_duration(summary.duration)));

        msg
    }

    fn format_action(action: &TaskAction) -> String {
        match action {
            TaskAction::FileCreated { path, lines } => {
                format!("Created: <code>{}</code> ({} lines)", Self::html_escape(path), lines)
            }
            TaskAction::FileModified { path, added, removed } => {
                format!(
                    "Modified: <code>{}</code> (+{} -{} lines)",
                    Self::html_escape(path), added, removed
                )
            }
            TaskAction::FileDeleted { path } => {
                format!("Deleted: <code>{}</code>", Self::html_escape(path))
            }
            TaskAction::GitCommit { hash, message } => {
                let short_hash = &hash[..7.min(hash.len())];
                let msg = Self::truncate(message, 50);
                format!("Commit: <code>{}</code> \"{}\"", short_hash, Self::html_escape(&msg))
            }
            TaskAction::GitBranch { name, action } => {
                format!("Branch {}: <code>{}</code>", action.as_str(), Self::html_escape(name))
            }
            TaskAction::GitPush { branch, remote } => {
                format!("Pushed: <code>{}</code> to {}", Self::html_escape(branch), remote)
            }
            TaskAction::TestsRan { passed, failed, skipped } => {
                let status = if *failed == 0 { "Passed" } else { "Failed" };
                format!("Tests {}: {} passed, {} failed, {} skipped", status, passed, failed, skipped)
            }
            TaskAction::BuildCompleted { artifact, size_bytes } => {
                let size = size_bytes
                    .map(|s| format!(" ({})", Self::format_size(s)))
                    .unwrap_or_default();
                format!("Built: <code>{}</code>{}", Self::html_escape(artifact), size)
            }
            TaskAction::CommandRan { cmd, exit_code } => {
                let status = if *exit_code == 0 { "OK" } else { "Failed" };
                format!(
                    "{}: <code>{}</code> (exit: {})",
                    status, Self::html_escape(&Self::truncate(cmd, 35)), exit_code
                )
            }
        }
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    fn format_size(bytes: u64) -> String {
        if bytes < 1024 {
            format!("{} B", bytes)
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }

    fn html_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn truncate(s: &str, max: usize) -> String {
        if s.len() <= max {
            s.to_string()
        } else {
            format!("{}...", &s[..max.saturating_sub(3)])
        }
    }

    fn truncate_diff(diff: &str, max_chars: usize) -> String {
        if diff.len() <= max_chars {
            diff.to_string()
        } else {
            let lines: Vec<&str> = diff.lines().take(15).collect();
            let truncated = lines.join("\n");
            let remaining = diff.lines().count().saturating_sub(15);
            if remaining > 0 {
                format!("{}\n... ({} more lines)", truncated, remaining)
            } else {
                truncated
            }
        }
    }

    /// Format error message with optional hint
    pub fn format_error(error: &str, hint: Option<&str>) -> String {
        let mut msg = format!("<b>Error:</b> {}\n", Self::html_escape(error));
        if let Some(h) = hint {
            msg.push_str(&format!("\n<b>Hint:</b> {}", Self::html_escape(h)));
        }
        msg
    }

    /// Format timeout message
    pub fn format_timeout(elapsed: Duration, last_output: Option<&str>) -> String {
        let mut msg = format!(
            "<b>Operation timed out</b>\n\nNo response for {} seconds.\n",
            elapsed.as_secs()
        );
        if let Some(output) = last_output {
            let preview = Self::truncate(output, 200);
            msg.push_str(&format!("\nLast output:\n<pre>{}</pre>", Self::html_escape(&preview)));
        }
        msg.push_str("\n\nThe process may be stuck waiting for input or credentials.");
        msg
    }
}

/// Parse Claude Code output to extract actions
pub struct OutputParser;

// Regex patterns
static RE_FILE_WRITE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:wrote|created|writing|write)\s+(?:to\s+)?(\S+\.\w+)").unwrap()
});

static RE_FILE_MODIFY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:modified|updated|edited|editing)\s+(\S+\.\w+)").unwrap()
});

static RE_GIT_COMMIT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[([a-f0-9]{7,40})\]\s+(.+)").unwrap()
});

static RE_GIT_PUSH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)push(?:ed|ing)?\s+(?:to\s+)?(\S+)").unwrap()
});

static RE_TEST_RESULT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d+)\s+passed").unwrap()
});

static RE_TEST_FAILED: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d+)\s+failed").unwrap()
});

static RE_BRANCH_CREATE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:created|switched to|checkout -b)\s+(?:branch\s+)?(\S+)").unwrap()
});

impl OutputParser {
    /// Parse Claude Code output and extract actions
    pub fn parse(output: &str) -> Vec<TaskAction> {
        let mut actions = Vec::new();
        let mut seen_files = std::collections::HashSet::new();

        for line in output.lines() {
            // File created
            if let Some(cap) = RE_FILE_WRITE.captures(line) {
                let path = cap[1].to_string();
                if seen_files.insert(path.clone()) {
                    actions.push(TaskAction::FileCreated { path, lines: 0 });
                }
            }

            // File modified
            if let Some(cap) = RE_FILE_MODIFY.captures(line) {
                let path = cap[1].to_string();
                if seen_files.insert(path.clone()) {
                    actions.push(TaskAction::FileModified {
                        path,
                        added: 0,
                        removed: 0,
                    });
                }
            }

            // Git commit
            if let Some(cap) = RE_GIT_COMMIT.captures(line) {
                actions.push(TaskAction::GitCommit {
                    hash: cap[1].to_string(),
                    message: cap[2].to_string(),
                });
            }

            // Git push
            if let Some(cap) = RE_GIT_PUSH.captures(line) {
                actions.push(TaskAction::GitPush {
                    branch: cap[1].to_string(),
                    remote: "origin".into(),
                });
            }

            // Branch creation
            if let Some(cap) = RE_BRANCH_CREATE.captures(line) {
                actions.push(TaskAction::GitBranch {
                    name: cap[1].to_string(),
                    action: BranchAction::Created,
                });
            }

            // Test results
            if let Some(passed_cap) = RE_TEST_RESULT.captures(line) {
                let passed: usize = passed_cap[1].parse().unwrap_or(0);
                let failed: usize = RE_TEST_FAILED
                    .captures(line)
                    .map(|c| c[1].parse().unwrap_or(0))
                    .unwrap_or(0);
                actions.push(TaskAction::TestsRan {
                    passed,
                    failed,
                    skipped: 0,
                });
            }

            // Build completed
            if line.contains("Finished") && (line.contains("release") || line.contains("debug")) {
                let artifact = if line.contains("release") {
                    "release build"
                } else {
                    "debug build"
                };
                actions.push(TaskAction::BuildCompleted {
                    artifact: artifact.into(),
                    size_bytes: None,
                });
            }
        }

        // Limit actions
        actions.truncate(15);
        actions
    }

    /// Extract error hint from error message
    pub fn extract_error_hint(error: &str) -> Option<String> {
        let error_lower = error.to_lowercase();

        if error_lower.contains("not found") && error_lower.contains("gh") {
            return Some("Install GitHub CLI: apt install gh && gh auth login".into());
        }
        if error_lower.contains("permission denied") {
            return Some("Check file permissions or try with sudo".into());
        }
        if error_lower.contains("authentication") || error_lower.contains("401") {
            return Some("Credentials may have expired. Check with /creds test".into());
        }
        if error_lower.contains("timeout") {
            return Some("The operation took too long. Try a simpler task.".into());
        }
        if error_lower.contains("not installed") || error_lower.contains("command not found") {
            return Some("A required tool is missing. Use /preflight to check.".into());
        }
        if error_lower.contains("merge conflict") {
            return Some("Git merge conflict. Review changes manually.".into());
        }

        None
    }
}

impl TaskSummary {
    /// Create a success summary from output
    pub fn from_output(output: &str, duration: Duration, exit_code: i32) -> Self {
        let actions = OutputParser::parse(output);
        Self {
            success: exit_code == 0,
            duration,
            actions,
            warnings: vec![],
            errors: if exit_code != 0 {
                vec![format!("Process exited with code {}", exit_code)]
            } else {
                vec![]
            },
            code_diff: None,
            exit_code: Some(exit_code),
        }
    }

    /// Create an error summary
    pub fn error(error: String, duration: Duration) -> Self {
        Self {
            success: false,
            duration,
            actions: vec![],
            warnings: vec![],
            errors: vec![error],
            code_diff: None,
            exit_code: None,
        }
    }

    /// Create a timeout summary
    pub fn timeout(duration: Duration, last_output: Option<String>) -> Self {
        let mut summary = Self {
            success: false,
            duration,
            actions: vec![],
            warnings: vec![],
            errors: vec!["Operation timed out".into()],
            code_diff: None,
            exit_code: None,
        };

        if let Some(output) = last_output {
            summary.actions = OutputParser::parse(&output);
        }

        summary
    }

    /// Add a warning
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_file_created() {
        let output = "Created file src/main.rs with 50 lines";
        let actions = OutputParser::parse(output);
        assert!(!actions.is_empty());
    }

    #[test]
    fn test_parse_git_commit() {
        let output = "[abc1234] Add new feature";
        let actions = OutputParser::parse(output);
        assert!(!actions.is_empty());
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(TaskFeedback::format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(TaskFeedback::format_duration(Duration::from_secs(125)), "2m 5s");
    }
}
