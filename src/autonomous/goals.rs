//! Multi-Turn Goal Tracking
//!
//! Tracks ongoing tasks and goals across conversation sessions:
//! - Automatic goal extraction from conversations
//! - Progress tracking and status updates
//! - Context restoration for resumed sessions
//! - Deadline and reminder management
//!
//! Industry standard: Intent persistence with state machine tracking

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Goal status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    /// Goal is active and being worked on
    Active,
    /// Goal is paused/deferred
    Paused,
    /// Goal is completed
    Completed,
    /// Goal was abandoned
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Abandoned => "abandoned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "active" => Some(GoalStatus::Active),
            "paused" => Some(GoalStatus::Paused),
            "completed" => Some(GoalStatus::Completed),
            "abandoned" => Some(GoalStatus::Abandoned),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, GoalStatus::Active | GoalStatus::Paused)
    }
}

/// Priority level for goals
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    Low = 0,
    Medium = 1,
    High = 2,
    Critical = 3,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Medium
    }
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Critical => "critical",
        }
    }
}

/// A tracked goal or task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub user_id: i64,
    pub description: String,
    pub status: GoalStatus,
    pub priority: Priority,
    pub created_at: i64,
    pub updated_at: i64,
    /// Progress notes and updates
    pub notes: Vec<GoalNote>,
    /// Related memory IDs
    pub related_memories: Vec<String>,
    /// Parent goal ID (for sub-goals)
    pub parent_id: Option<String>,
    /// Child goal IDs
    pub child_ids: Vec<String>,
    /// Deadline (Unix timestamp)
    pub deadline: Option<i64>,
    /// Tags for categorization
    pub tags: Vec<String>,
}

/// A progress note on a goal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalNote {
    pub content: String,
    pub timestamp: i64,
    pub auto_generated: bool,
}

impl Goal {
    /// Create a new goal
    pub fn new(user_id: i64, description: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            description: description.to_string(),
            status: GoalStatus::Active,
            priority: Priority::default(),
            created_at: now,
            updated_at: now,
            notes: vec![],
            related_memories: vec![],
            parent_id: None,
            child_ids: vec![],
            deadline: None,
            tags: vec![],
        }
    }

    /// Add a progress note
    pub fn add_note(&mut self, content: &str, auto_generated: bool) {
        self.notes.push(GoalNote {
            content: content.to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            auto_generated,
        });
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Update status
    pub fn set_status(&mut self, status: GoalStatus) {
        self.status = status;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Format goal for display
    pub fn format(&self) -> String {
        let mut s = format!(
            "[{}] {} ({})",
            self.status.as_str().to_uppercase(),
            self.description,
            self.priority.as_str()
        );

        if let Some(deadline) = self.deadline {
            let dt = chrono::DateTime::from_timestamp(deadline, 0)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".to_string());
            s.push_str(&format!(" - due: {}", dt));
        }

        if !self.notes.is_empty() {
            if let Some(last) = self.notes.last() {
                s.push_str(&format!("\n  └─ {}", last.content));
            }
        }

        s
    }
}

/// Goal tracker for managing multi-turn goals
pub struct GoalTracker {
    /// Goals indexed by ID
    goals: Arc<RwLock<HashMap<String, Goal>>>,
    /// Index: user_id -> goal_ids
    user_goals: Arc<RwLock<HashMap<i64, Vec<String>>>>,
}

impl GoalTracker {
    /// Create a new goal tracker
    pub fn new() -> Self {
        Self {
            goals: Arc::new(RwLock::new(HashMap::new())),
            user_goals: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new goal for a user
    pub async fn create_goal(&self, user_id: i64, description: &str) -> Goal {
        let goal = Goal::new(user_id, description);
        let id = goal.id.clone();

        // Store goal
        self.goals.write().await.insert(id.clone(), goal.clone());

        // Update user index
        self.user_goals
            .write()
            .await
            .entry(user_id)
            .or_default()
            .push(id);

        info!("Created goal: {} for user {}", goal.description, user_id);
        goal
    }

    /// Get all active goals for a user
    pub async fn get_active_goals(&self, user_id: i64) -> Vec<Goal> {
        let user_goals = self.user_goals.read().await;
        let goals = self.goals.read().await;

        user_goals
            .get(&user_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| goals.get(id))
                    .filter(|g| g.status.is_open())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all goals for a user (including completed)
    pub async fn get_all_goals(&self, user_id: i64) -> Vec<Goal> {
        let user_goals = self.user_goals.read().await;
        let goals = self.goals.read().await;

        user_goals
            .get(&user_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| goals.get(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get a specific goal by ID
    pub async fn get_goal(&self, goal_id: &str) -> Option<Goal> {
        self.goals.read().await.get(goal_id).cloned()
    }

    /// Update goal status
    pub async fn update_status(&self, goal_id: &str, status: GoalStatus) -> Option<Goal> {
        let mut goals = self.goals.write().await;
        if let Some(goal) = goals.get_mut(goal_id) {
            goal.set_status(status);
            Some(goal.clone())
        } else {
            None
        }
    }

    /// Add a note to a goal
    pub async fn add_note(&self, goal_id: &str, note: &str, auto_generated: bool) -> Option<Goal> {
        let mut goals = self.goals.write().await;
        if let Some(goal) = goals.get_mut(goal_id) {
            goal.add_note(note, auto_generated);
            Some(goal.clone())
        } else {
            None
        }
    }

    /// Extract goals from a user message
    ///
    /// Looks for patterns indicating task intentions:
    /// - "I need to..."
    /// - "TODO: ..."
    /// - "Remind me to..."
    /// - "I want to..."
    /// - "Let's work on..."
    pub async fn extract_goals(&self, message: &str, user_id: i64) -> Vec<Goal> {
        let lower = message.to_lowercase();
        let mut extracted = Vec::new();

        // Goal extraction patterns
        let patterns = [
            ("i need to ", Priority::Medium),
            ("todo:", Priority::Medium),
            ("remind me to ", Priority::Low),
            ("i want to ", Priority::Low),
            ("let's work on ", Priority::Medium),
            ("we should ", Priority::Low),
            ("i have to ", Priority::High),
            ("urgent:", Priority::Critical),
            ("must ", Priority::High),
        ];

        for (pattern, priority) in patterns {
            if let Some(pos) = lower.find(pattern) {
                let start = pos + pattern.len();
                let content = &message[start..];

                // Extract until end of sentence
                let end = content
                    .find(|c: char| c == '.' || c == '!' || c == '?' || c == '\n')
                    .unwrap_or(content.len().min(200));

                let description = content[..end].trim().to_string();

                if description.len() >= 5 {
                    let mut goal = self.create_goal(user_id, &description).await;
                    goal.priority = priority;

                    // Re-save with updated priority
                    self.goals.write().await.insert(goal.id.clone(), goal.clone());

                    extracted.push(goal);
                }
            }
        }

        extracted
    }

    /// Detect if a message indicates goal completion
    pub async fn detect_completion(&self, message: &str, user_id: i64) -> Vec<String> {
        let lower = message.to_lowercase();
        let completion_patterns = ["done", "finished", "completed", "fixed", "resolved", "closed"];

        if !completion_patterns.iter().any(|p| lower.contains(p)) {
            return vec![];
        }

        // Get active goals and check for matches
        let active = self.get_active_goals(user_id).await;
        let mut completed = Vec::new();

        for goal in active {
            // Check if goal description terms appear in message
            let goal_terms: Vec<&str> = goal
                .description
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .collect();

            let matches = goal_terms.iter().filter(|t| lower.contains(&t.to_lowercase())).count();

            if matches >= 2 || (goal_terms.len() <= 3 && matches >= 1) {
                completed.push(goal.id);
            }
        }

        completed
    }

    /// Auto-complete goals detected in message
    pub async fn auto_complete(&self, message: &str, user_id: i64) -> Vec<Goal> {
        let goal_ids = self.detect_completion(message, user_id).await;
        let mut completed = Vec::new();

        for id in goal_ids {
            if let Some(goal) = self.update_status(&id, GoalStatus::Completed).await {
                self.add_note(&id, &format!("Auto-completed: {}", message), true).await;
                completed.push(goal);
            }
        }

        completed
    }

    /// Format goals summary for context
    pub async fn format_goals_context(&self, user_id: i64) -> String {
        let active = self.get_active_goals(user_id).await;

        if active.is_empty() {
            return String::new();
        }

        let mut s = String::from("Active tasks:\n");
        for goal in active.iter().take(5) {
            s.push_str(&format!("- {}\n", goal.description));
        }

        if active.len() > 5 {
            s.push_str(&format!("... and {} more\n", active.len() - 5));
        }

        s
    }
}

impl Default for GoalTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_goal_creation() {
        let tracker = GoalTracker::new();
        let goal = tracker.create_goal(123, "Fix the login bug").await;

        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.user_id, 123);
        assert!(goal.description.contains("login"));
    }

    #[tokio::test]
    async fn test_goal_extraction() {
        let tracker = GoalTracker::new();

        let message = "I need to fix the authentication bug before tomorrow";
        let goals = tracker.extract_goals(message, 123).await;

        assert_eq!(goals.len(), 1);
        assert!(goals[0].description.contains("authentication"));
    }

    #[tokio::test]
    async fn test_goal_status_update() {
        let tracker = GoalTracker::new();
        let goal = tracker.create_goal(123, "Test task").await;

        let updated = tracker.update_status(&goal.id, GoalStatus::Completed).await;
        assert!(updated.is_some());
        assert_eq!(updated.unwrap().status, GoalStatus::Completed);
    }
}
