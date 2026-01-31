//! Multi-Turn Goal Tracking with SQLite Persistence
//!
//! Tracks ongoing tasks and goals across conversation sessions:
//! - Automatic goal extraction from conversations
//! - Progress tracking and status updates
//! - SQLite persistence for durability across restarts
//! - Context restoration for resumed sessions
//!
//! Industry standard: Intent persistence with state machine tracking

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
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

    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Priority::Low,
            1 => Priority::Medium,
            2 => Priority::High,
            3 => Priority::Critical,
            _ => Priority::Medium,
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
    /// Latest progress note
    pub last_note: Option<String>,
    /// Parent goal ID (for sub-goals)
    pub parent_id: Option<String>,
    /// Deadline (Unix timestamp)
    pub deadline: Option<i64>,
    /// Tags for categorization (comma-separated in DB)
    pub tags: Vec<String>,
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
            last_note: None,
            parent_id: None,
            deadline: None,
            tags: vec![],
        }
    }

    /// Format goal for display
    pub fn format(&self) -> String {
        let status_icon = match self.status {
            GoalStatus::Active => "🔵",
            GoalStatus::Paused => "⏸️",
            GoalStatus::Completed => "✅",
            GoalStatus::Abandoned => "❌",
        };

        let priority_icon = match self.priority {
            Priority::Low => "",
            Priority::Medium => "",
            Priority::High => "❗",
            Priority::Critical => "🔥",
        };

        let mut s = format!(
            "{} {}{} ({})",
            status_icon,
            priority_icon,
            self.description,
            self.priority.as_str()
        );

        if let Some(deadline) = self.deadline {
            let dt = chrono::DateTime::from_timestamp(deadline, 0)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".to_string());
            s.push_str(&format!(" - due: {}", dt));
        }

        if let Some(ref note) = self.last_note {
            s.push_str(&format!("\n  └─ {}", note));
        }

        s
    }

    /// Short format for listings
    pub fn format_short(&self) -> String {
        let status_icon = match self.status {
            GoalStatus::Active => "●",
            GoalStatus::Paused => "○",
            GoalStatus::Completed => "✓",
            GoalStatus::Abandoned => "✗",
        };
        format!("{} {}", status_icon, self.description)
    }
}

/// Goal tracker with SQLite persistence
pub struct GoalTracker {
    conn: Mutex<Connection>,
}

impl GoalTracker {
    /// Create a new goal tracker with SQLite persistence
    pub fn new() -> Self {
        Self::open(":memory:").expect("Failed to create in-memory goal tracker")
    }

    /// Open goal tracker with a specific database path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let tracker = Self { conn: Mutex::new(conn) };
        tracker.init_schema()?;
        Ok(tracker)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                description TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                priority INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                last_note TEXT,
                parent_id TEXT,
                deadline INTEGER,
                tags TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_goals_user ON goals(user_id);
            CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);
            CREATE INDEX IF NOT EXISTS idx_goals_user_status ON goals(user_id, status);
            "#,
        )?;
        Ok(())
    }

    /// Create a new goal for a user
    pub async fn create_goal(&self, user_id: i64, description: &str) -> Goal {
        let goal = Goal::new(user_id, description);
        if let Err(e) = self.save_goal(&goal) {
            tracing::warn!("Failed to persist goal: {}", e);
        }
        info!("Created goal: {} for user {}", goal.description, user_id);
        goal
    }

    /// Save a goal to the database
    fn save_goal(&self, goal: &Goal) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO goals
            (id, user_id, description, status, priority, created_at, updated_at, last_note, parent_id, deadline, tags)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                goal.id,
                goal.user_id,
                goal.description,
                goal.status.as_str(),
                goal.priority as i32,
                goal.created_at,
                goal.updated_at,
                goal.last_note,
                goal.parent_id,
                goal.deadline,
                goal.tags.join(","),
            ],
        )?;
        Ok(())
    }

    /// Load a goal from the database
    fn load_goal(row: &rusqlite::Row) -> rusqlite::Result<Goal> {
        let status_str: String = row.get(3)?;
        let priority_int: i32 = row.get(4)?;
        let tags_str: Option<String> = row.get(10)?;

        Ok(Goal {
            id: row.get(0)?,
            user_id: row.get(1)?,
            description: row.get(2)?,
            status: GoalStatus::parse(&status_str).unwrap_or(GoalStatus::Active),
            priority: Priority::from_i32(priority_int),
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            last_note: row.get(7)?,
            parent_id: row.get(8)?,
            deadline: row.get(9)?,
            tags: tags_str
                .map(|s| s.split(',').filter(|t| !t.is_empty()).map(String::from).collect())
                .unwrap_or_default(),
        })
    }

    /// Get all active goals for a user
    pub async fn get_active_goals(&self, user_id: i64) -> Vec<Goal> {
        self.get_goals_by_status(user_id, &["active", "paused"])
            .unwrap_or_default()
    }

    /// Get all goals for a user (including completed)
    pub async fn get_all_goals(&self, user_id: i64) -> Vec<Goal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut stmt = match conn.prepare(
            "SELECT id, user_id, description, status, priority, created_at, updated_at,
                    last_note, parent_id, deadline, tags
             FROM goals WHERE user_id = ?1 ORDER BY updated_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map(params![user_id], Self::load_goal)
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// Get goals by status
    fn get_goals_by_status(&self, user_id: i64, statuses: &[&str]) -> Result<Vec<Goal>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let placeholders = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, user_id, description, status, priority, created_at, updated_at,
                    last_note, parent_id, deadline, tags
             FROM goals WHERE user_id = ?1 AND status IN ({}) ORDER BY priority DESC, updated_at DESC",
            placeholders
        );

        let mut stmt = conn.prepare(&sql)?;

        // Build params dynamically
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(user_id)];
        for s in statuses {
            params_vec.push(Box::new(s.to_string()));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

        let goals = stmt
            .query_map(params_refs.as_slice(), Self::load_goal)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(goals)
    }

    /// Get a specific goal by ID
    pub async fn get_goal(&self, goal_id: &str) -> Option<Goal> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT id, user_id, description, status, priority, created_at, updated_at,
                    last_note, parent_id, deadline, tags
             FROM goals WHERE id = ?1",
            params![goal_id],
            Self::load_goal,
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Update goal status
    pub async fn update_status(&self, goal_id: &str, status: GoalStatus) -> Option<Goal> {
        let now = chrono::Utc::now().timestamp();
        {
            let conn = self.conn.lock().ok()?;
            conn.execute(
                "UPDATE goals SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, goal_id],
            )
            .ok()?;
        }
        self.get_goal(goal_id).await
    }

    /// Add a note to a goal
    pub async fn add_note(&self, goal_id: &str, note: &str) -> Option<Goal> {
        let now = chrono::Utc::now().timestamp();
        {
            let conn = self.conn.lock().ok()?;
            conn.execute(
                "UPDATE goals SET last_note = ?1, updated_at = ?2 WHERE id = ?3",
                params![note, now, goal_id],
            )
            .ok()?;
        }
        self.get_goal(goal_id).await
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
                    if let Err(e) = self.save_goal(&goal) {
                        tracing::warn!("Failed to update goal priority: {}", e);
                    }

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
                let _ = self.add_note(&id, &format!("Auto-completed: {}", message)).await;
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

    /// Get goal statistics for a user
    pub fn get_stats(&self, user_id: i64) -> Result<GoalStats> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM goals WHERE user_id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;

        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM goals WHERE user_id = ?1 AND status = 'active'",
            params![user_id],
            |row| row.get(0),
        )?;

        let completed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM goals WHERE user_id = ?1 AND status = 'completed'",
            params![user_id],
            |row| row.get(0),
        )?;

        let paused: i64 = conn.query_row(
            "SELECT COUNT(*) FROM goals WHERE user_id = ?1 AND status = 'paused'",
            params![user_id],
            |row| row.get(0),
        )?;

        Ok(GoalStats {
            total: total as usize,
            active: active as usize,
            completed: completed as usize,
            paused: paused as usize,
        })
    }
}

/// Goal statistics
#[derive(Debug, Clone, Default)]
pub struct GoalStats {
    pub total: usize,
    pub active: usize,
    pub completed: usize,
    pub paused: usize,
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
    async fn test_goal_persistence() {
        let tracker = GoalTracker::new();
        let goal = tracker.create_goal(123, "Test persistence").await;

        // Fetch it back
        let fetched = tracker.get_goal(&goal.id).await;
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().description, "Test persistence");
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

    #[tokio::test]
    async fn test_goal_stats() {
        let tracker = GoalTracker::new();
        tracker.create_goal(123, "Active goal 1").await;
        tracker.create_goal(123, "Active goal 2").await;
        let goal3 = tracker.create_goal(123, "Completed goal").await;
        tracker.update_status(&goal3.id, GoalStatus::Completed).await;

        let stats = tracker.get_stats(123).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.active, 2);
        assert_eq!(stats.completed, 1);
    }
}
