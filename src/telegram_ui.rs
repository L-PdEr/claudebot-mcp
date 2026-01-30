//! Telegram UI Components
//!
//! Advanced UI features for Telegram bot:
//! - Inline keyboard buttons
//! - Live-updating progress messages
//! - Interactive task controls
//! - Context-aware responses

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, MessageId,
};
use tokio::sync::RwLock;

// ============ Inline Keyboards ============

/// Button action types for callbacks
#[derive(Debug, Clone)]
pub enum ButtonAction {
    ViewLogs(String),      // task_id
    PauseTask(String),     // task_id
    ResumeTask(String),    // task_id
    CancelTask(String),    // task_id
    RetryTask(String),     // task_id
    ShowDiff,
    ShowError,
    Confirm(String),       // action_id
    Deny(String),          // action_id
    SelectOption(String),  // option_id
}

impl ButtonAction {
    /// Encode action as callback data string
    pub fn encode(&self) -> String {
        match self {
            Self::ViewLogs(id) => format!("logs:{}", id),
            Self::PauseTask(id) => format!("pause:{}", id),
            Self::ResumeTask(id) => format!("resume:{}", id),
            Self::CancelTask(id) => format!("cancel:{}", id),
            Self::RetryTask(id) => format!("retry:{}", id),
            Self::ShowDiff => "show:diff".to_string(),
            Self::ShowError => "show:error".to_string(),
            Self::Confirm(id) => format!("confirm:{}", id),
            Self::Deny(id) => format!("deny:{}", id),
            Self::SelectOption(id) => format!("select:{}", id),
        }
    }

    /// Decode callback data string to action
    pub fn decode(data: &str) -> Option<Self> {
        let parts: Vec<&str> = data.splitn(2, ':').collect();
        if parts.len() != 2 {
            return None;
        }

        let (action, id) = (parts[0], parts[1].to_string());
        match action {
            "logs" => Some(Self::ViewLogs(id)),
            "pause" => Some(Self::PauseTask(id)),
            "resume" => Some(Self::ResumeTask(id)),
            "cancel" => Some(Self::CancelTask(id)),
            "retry" => Some(Self::RetryTask(id)),
            "show" if id == "diff" => Some(Self::ShowDiff),
            "show" if id == "error" => Some(Self::ShowError),
            "confirm" => Some(Self::Confirm(id)),
            "deny" => Some(Self::Deny(id)),
            "select" => Some(Self::SelectOption(id)),
            _ => None,
        }
    }
}

/// Build inline keyboard for task progress
pub fn task_progress_keyboard(task_id: &str, is_running: bool) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();

    // First row: Logs and Diff
    rows.push(vec![
        InlineKeyboardButton::callback("Logs", ButtonAction::ViewLogs(task_id.to_string()).encode()),
        InlineKeyboardButton::callback("Diff", ButtonAction::ShowDiff.encode()),
    ]);

    // Second row: Control buttons
    if is_running {
        rows.push(vec![
            InlineKeyboardButton::callback("Pause", ButtonAction::PauseTask(task_id.to_string()).encode()),
            InlineKeyboardButton::callback("Cancel", ButtonAction::CancelTask(task_id.to_string()).encode()),
        ]);
    } else {
        rows.push(vec![
            InlineKeyboardButton::callback("Retry", ButtonAction::RetryTask(task_id.to_string()).encode()),
        ]);
    }

    InlineKeyboardMarkup::new(rows)
}

/// Build confirmation keyboard
pub fn confirmation_keyboard(action_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Confirm", ButtonAction::Confirm(action_id.to_string()).encode()),
        InlineKeyboardButton::callback("Cancel", ButtonAction::Deny(action_id.to_string()).encode()),
    ]])
}

/// Build options keyboard
pub fn options_keyboard(options: &[(&str, &str)]) -> InlineKeyboardMarkup {
    let buttons: Vec<Vec<InlineKeyboardButton>> = options
        .iter()
        .map(|(label, id)| {
            vec![InlineKeyboardButton::callback(
                *label,
                ButtonAction::SelectOption(id.to_string()).encode(),
            )]
        })
        .collect();

    InlineKeyboardMarkup::new(buttons)
}

/// Build worker control keyboard
pub fn worker_keyboard(worker_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("Logs", format!("wlogs:{}", worker_id)),
            InlineKeyboardButton::callback("Status", format!("wstatus:{}", worker_id)),
        ],
        vec![
            InlineKeyboardButton::callback("Kill", format!("wkill:{}", worker_id)),
        ],
    ])
}

// ============ Progress Messages ============

/// Progress step status
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl StepStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::Running => "[~]",
            Self::Completed => "[x]",
            Self::Failed => "[!]",
            Self::Skipped => "[-]",
        }
    }
}

/// A step in a progress sequence
#[derive(Debug, Clone)]
pub struct ProgressStep {
    pub name: String,
    pub status: StepStatus,
    pub detail: Option<String>,
}

impl ProgressStep {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            status: StepStatus::Pending,
            detail: None,
        }
    }

    pub fn with_status(mut self, status: StepStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(detail.to_string());
        self
    }
}

/// Progress tracker for a task
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    pub task_id: String,
    pub title: String,
    pub steps: Vec<ProgressStep>,
    pub current_step: usize,
    pub percent: f64,
    pub started_at: Instant,
    pub message_id: Option<MessageId>,
    pub chat_id: Option<ChatId>,
}

impl ProgressTracker {
    pub fn new(task_id: &str, title: &str, steps: Vec<&str>) -> Self {
        Self {
            task_id: task_id.to_string(),
            title: title.to_string(),
            steps: steps.iter().map(|s| ProgressStep::new(s)).collect(),
            current_step: 0,
            percent: 0.0,
            started_at: Instant::now(),
            message_id: None,
            chat_id: None,
        }
    }

    /// Advance to next step
    pub fn advance(&mut self) {
        if self.current_step < self.steps.len() {
            self.steps[self.current_step].status = StepStatus::Completed;
            self.current_step += 1;
            if self.current_step < self.steps.len() {
                self.steps[self.current_step].status = StepStatus::Running;
            }
            self.update_percent();
        }
    }

    /// Mark current step with detail
    pub fn set_detail(&mut self, detail: &str) {
        if self.current_step < self.steps.len() {
            self.steps[self.current_step].detail = Some(detail.to_string());
        }
    }

    /// Mark current step as failed
    pub fn fail(&mut self, reason: &str) {
        if self.current_step < self.steps.len() {
            self.steps[self.current_step].status = StepStatus::Failed;
            self.steps[self.current_step].detail = Some(reason.to_string());
        }
    }

    /// Complete all remaining steps
    pub fn complete(&mut self) {
        for step in &mut self.steps {
            if step.status == StepStatus::Pending || step.status == StepStatus::Running {
                step.status = StepStatus::Completed;
            }
        }
        self.percent = 100.0;
    }

    fn update_percent(&mut self) {
        if self.steps.is_empty() {
            self.percent = 0.0;
        } else {
            self.percent = (self.current_step as f64 / self.steps.len() as f64) * 100.0;
        }
    }

    /// Format progress message
    pub fn format(&self) -> String {
        let mut msg = format!("<b>{}</b>\n\n", html_escape(&self.title));

        // Steps
        for step in &self.steps {
            msg.push_str(&format!("{} {}", step.status.icon(), html_escape(&step.name)));
            if let Some(ref detail) = step.detail {
                msg.push_str(&format!("\n   <i>{}</i>", html_escape(detail)));
            }
            msg.push('\n');
        }

        // Progress bar
        msg.push('\n');
        msg.push_str(&format_progress_bar(self.percent, 20));
        msg.push_str(&format!(" {:.0}%\n", self.percent));

        // Duration
        let elapsed = self.started_at.elapsed();
        msg.push_str(&format!("\nDuration: {}", format_duration(elapsed)));

        msg
    }
}

/// Format a progress bar
pub fn format_progress_bar(percent: f64, width: usize) -> String {
    let filled = ((percent / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "=".repeat(filled), " ".repeat(empty))
}

/// Format duration for display
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// HTML escape for Telegram
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============ Context Understanding ============

/// Context for understanding natural language references
#[derive(Debug, Clone, Default)]
pub struct ConversationContext {
    pub last_file: Option<String>,
    pub last_error: Option<String>,
    pub last_command: Option<String>,
    pub last_task_id: Option<String>,
    pub last_diff: Option<String>,
    pub last_mentioned_files: Vec<String>,
    pub pending_confirmation: Option<String>,
}

impl ConversationContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update context after a file operation
    pub fn set_file(&mut self, path: &str) {
        self.last_file = Some(path.to_string());
        if !self.last_mentioned_files.contains(&path.to_string()) {
            self.last_mentioned_files.push(path.to_string());
            if self.last_mentioned_files.len() > 10 {
                self.last_mentioned_files.remove(0);
            }
        }
    }

    /// Update context after an error
    pub fn set_error(&mut self, error: &str) {
        self.last_error = Some(error.to_string());
    }

    /// Update context after a command
    pub fn set_command(&mut self, command: &str) {
        self.last_command = Some(command.to_string());
    }

    /// Update context after starting a task
    pub fn set_task(&mut self, task_id: &str) {
        self.last_task_id = Some(task_id.to_string());
    }

    /// Update context with diff
    pub fn set_diff(&mut self, diff: &str) {
        self.last_diff = Some(diff.to_string());
    }

    /// Clear pending confirmation
    pub fn clear_confirmation(&mut self) {
        self.pending_confirmation = None;
    }
}

/// Parse natural language references
pub struct ContextParser;

impl ContextParser {
    /// Expand references in user input using context
    pub fn expand(input: &str, ctx: &ConversationContext) -> String {
        let mut result = input.to_string();
        let lower = input.to_lowercase();

        // "that file" / "the file" -> last file
        if (lower.contains("that file") || lower.contains("the file")) && ctx.last_file.is_some() {
            let file = ctx.last_file.as_ref().unwrap();
            result = result
                .replace("that file", file)
                .replace("That file", file)
                .replace("the file", file)
                .replace("The file", file);
        }

        // "the error" / "that error" -> include error context
        if (lower.contains("the error") || lower.contains("that error")) && ctx.last_error.is_some() {
            let error = ctx.last_error.as_ref().unwrap();
            result = format!("{}\n\nContext - Last error:\n{}", result, error);
        }

        // "the diff" / "those changes" -> include diff context
        if (lower.contains("the diff") || lower.contains("those changes") || lower.contains("the changes"))
            && ctx.last_diff.is_some()
        {
            let diff = ctx.last_diff.as_ref().unwrap();
            result = format!("{}\n\nContext - Recent changes:\n{}", result, diff);
        }

        result
    }

    /// Detect intent from user input
    pub fn detect_intent(input: &str, ctx: &ConversationContext) -> Option<Intent> {
        let lower = input.to_lowercase().trim().to_string();

        // Retry/again
        if lower == "again" || lower == "retry" || lower == "try again" || lower == "redo" {
            if let Some(ref cmd) = ctx.last_command {
                return Some(Intent::Retry(cmd.clone()));
            }
        }

        // Fix it
        if lower == "fix it" || lower == "fix that" || lower == "fix the error" {
            if let Some(ref error) = ctx.last_error {
                return Some(Intent::FixError(error.clone()));
            }
        }

        // Cancel
        if lower == "cancel" || lower == "stop" || lower == "abort" {
            if let Some(ref task_id) = ctx.last_task_id {
                return Some(Intent::Cancel(task_id.clone()));
            }
        }

        // Yes/No for confirmations
        if ctx.pending_confirmation.is_some() {
            if lower == "yes" || lower == "y" || lower == "confirm" || lower == "ok" {
                return Some(Intent::Confirm);
            }
            if lower == "no" || lower == "n" || lower == "cancel" || lower == "deny" {
                return Some(Intent::Deny);
            }
        }

        // Show commands
        if lower == "show diff" || lower == "what changed" {
            return Some(Intent::ShowDiff);
        }
        if lower == "show error" || lower == "what went wrong" {
            return Some(Intent::ShowError);
        }
        if lower == "show logs" || lower == "logs" {
            return Some(Intent::ShowLogs);
        }

        None
    }
}

/// Detected user intent
#[derive(Debug, Clone)]
pub enum Intent {
    Retry(String),      // command to retry
    FixError(String),   // error to fix
    Cancel(String),     // task_id to cancel
    Confirm,
    Deny,
    ShowDiff,
    ShowError,
    ShowLogs,
}

// ============ Suggestions ============

/// Suggestion types
#[derive(Debug, Clone)]
pub enum Suggestion {
    RunTests,
    CommitChanges,
    PushBranch,
    CreatePR,
    FixError(String),
    ReviewChanges,
    Custom(String),
}

impl Suggestion {
    pub fn format(&self) -> String {
        match self {
            Self::RunTests => "Run tests to verify changes".to_string(),
            Self::CommitChanges => "Commit the changes".to_string(),
            Self::PushBranch => "Push to remote branch".to_string(),
            Self::CreatePR => "Create a pull request".to_string(),
            Self::FixError(e) => format!("Fix: {}", e),
            Self::ReviewChanges => "Review the diff before proceeding".to_string(),
            Self::Custom(s) => s.clone(),
        }
    }

    pub fn command(&self) -> Option<String> {
        match self {
            Self::RunTests => Some("/bypass cargo test".to_string()),
            Self::CommitChanges => Some("/bypass git add -A && git commit".to_string()),
            Self::PushBranch => Some("/bypass git push".to_string()),
            Self::CreatePR => Some("/bypass gh pr create".to_string()),
            Self::FixError(_) => None,
            Self::ReviewChanges => Some("/diff".to_string()),
            Self::Custom(_) => None,
        }
    }
}

/// Generate suggestions based on completed task
pub fn suggest_next_actions(task_description: &str, success: bool, has_changes: bool) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();
    let lower = task_description.to_lowercase();

    if !success {
        suggestions.push(Suggestion::FixError("Review the error and try again".to_string()));
        return suggestions;
    }

    if has_changes {
        suggestions.push(Suggestion::ReviewChanges);
    }

    // Code changes -> test -> commit
    if lower.contains("implement") || lower.contains("add") || lower.contains("create")
        || lower.contains("fix") || lower.contains("update")
    {
        if has_changes {
            suggestions.push(Suggestion::RunTests);
            suggestions.push(Suggestion::CommitChanges);
        }
    }

    // Commit -> push -> PR
    if lower.contains("commit") {
        suggestions.push(Suggestion::PushBranch);
    }

    if lower.contains("push") {
        suggestions.push(Suggestion::CreatePR);
    }

    // Limit suggestions
    suggestions.truncate(3);
    suggestions
}

/// Format suggestions as keyboard
pub fn suggestions_keyboard(suggestions: &[Suggestion]) -> Option<InlineKeyboardMarkup> {
    if suggestions.is_empty() {
        return None;
    }

    let buttons: Vec<Vec<InlineKeyboardButton>> = suggestions
        .iter()
        .filter_map(|s| {
            s.command().map(|cmd| {
                vec![InlineKeyboardButton::callback(
                    &s.format(),
                    format!("suggest:{}", cmd),
                )]
            })
        })
        .collect();

    if buttons.is_empty() {
        None
    } else {
        Some(InlineKeyboardMarkup::new(buttons))
    }
}

// ============ Progress Manager ============

/// Manages active progress trackers
pub struct ProgressManager {
    trackers: Arc<RwLock<HashMap<String, ProgressTracker>>>,
}

impl ProgressManager {
    pub fn new() -> Self {
        Self {
            trackers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new progress tracker
    pub async fn create(&self, task_id: &str, title: &str, steps: Vec<&str>) -> ProgressTracker {
        let tracker = ProgressTracker::new(task_id, title, steps);
        let mut trackers = self.trackers.write().await;
        trackers.insert(task_id.to_string(), tracker.clone());
        tracker
    }

    /// Get a progress tracker
    pub async fn get(&self, task_id: &str) -> Option<ProgressTracker> {
        let trackers = self.trackers.read().await;
        trackers.get(task_id).cloned()
    }

    /// Update a progress tracker
    pub async fn update<F>(&self, task_id: &str, f: F)
    where
        F: FnOnce(&mut ProgressTracker),
    {
        let mut trackers = self.trackers.write().await;
        if let Some(tracker) = trackers.get_mut(task_id) {
            f(tracker);
        }
    }

    /// Remove a progress tracker
    pub async fn remove(&self, task_id: &str) {
        let mut trackers = self.trackers.write().await;
        trackers.remove(task_id);
    }
}

impl Default for ProgressManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_action_encode_decode() {
        let action = ButtonAction::ViewLogs("task123".to_string());
        let encoded = action.encode();
        let decoded = ButtonAction::decode(&encoded);

        assert!(matches!(decoded, Some(ButtonAction::ViewLogs(id)) if id == "task123"));
    }

    #[test]
    fn test_progress_bar() {
        assert_eq!(format_progress_bar(0.0, 10), "[          ]");
        assert_eq!(format_progress_bar(50.0, 10), "[=====     ]");
        assert_eq!(format_progress_bar(100.0, 10), "[==========]");
    }

    #[test]
    fn test_context_expand() {
        let mut ctx = ConversationContext::new();
        ctx.set_file("/src/main.rs");
        ctx.set_error("compilation failed");

        let input = "fix that file";
        let expanded = ContextParser::expand(input, &ctx);
        assert!(expanded.contains("/src/main.rs"));

        let input2 = "explain the error";
        let expanded2 = ContextParser::expand(input2, &ctx);
        assert!(expanded2.contains("compilation failed"));
    }

    #[test]
    fn test_intent_detection() {
        let mut ctx = ConversationContext::new();
        ctx.set_command("cargo build");
        ctx.set_error("type mismatch");

        assert!(matches!(
            ContextParser::detect_intent("again", &ctx),
            Some(Intent::Retry(_))
        ));

        assert!(matches!(
            ContextParser::detect_intent("fix it", &ctx),
            Some(Intent::FixError(_))
        ));
    }

    #[test]
    fn test_progress_tracker() {
        let mut tracker = ProgressTracker::new("task1", "Building", vec!["Compile", "Test", "Deploy"]);

        assert_eq!(tracker.percent, 0.0);

        tracker.advance();
        assert!(tracker.percent > 0.0);
        assert_eq!(tracker.steps[0].status, StepStatus::Completed);
        assert_eq!(tracker.steps[1].status, StepStatus::Running);
    }

    #[test]
    fn test_suggestions() {
        let suggestions = suggest_next_actions("implement user auth", true, true);
        assert!(!suggestions.is_empty());
    }
}
