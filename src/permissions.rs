//! Permission System for Autonomous Coding
//!
//! Implements tiered access control for Claude Code operations:
//! - **Restricted**: Read-only, no file changes (default for sensitive repos)
//! - **Supervised**: Can propose changes, needs approval
//! - **Autonomous**: Full access, can commit and push
//!
//! Session-based escalation: User can grant temporary full access.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Permission level for operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Read-only access, no modifications allowed
    Restricted,
    /// Can propose changes, requires user approval
    Supervised,
    /// Full autonomous access, can commit/push
    Autonomous,
}

impl Default for PermissionLevel {
    fn default() -> Self {
        PermissionLevel::Supervised
    }
}

/// Operation types that require permission checks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Read files, search code
    Read,
    /// Create or modify files
    Write,
    /// Delete files
    Delete,
    /// Run shell commands
    Execute,
    /// Git commit
    Commit,
    /// Git push
    Push,
    /// Install dependencies
    Install,
    /// Deploy to production
    Deploy,
}

impl Operation {
    /// Minimum permission level required for this operation
    pub fn required_level(&self) -> PermissionLevel {
        match self {
            Operation::Read => PermissionLevel::Restricted,
            Operation::Write | Operation::Delete => PermissionLevel::Supervised,
            Operation::Execute | Operation::Commit => PermissionLevel::Supervised,
            Operation::Push | Operation::Install => PermissionLevel::Autonomous,
            Operation::Deploy => PermissionLevel::Autonomous,
        }
    }

    /// Risk level (0-10) for Llama evaluation
    pub fn risk_level(&self) -> u8 {
        match self {
            Operation::Read => 0,
            Operation::Write => 3,
            Operation::Delete => 5,
            Operation::Execute => 6,
            Operation::Commit => 4,
            Operation::Push => 7,
            Operation::Install => 6,
            Operation::Deploy => 10,
        }
    }
}

/// Project-specific permission configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPermissions {
    /// Base permission level for this project
    pub base_level: PermissionLevel,
    /// Specific overrides for paths (glob patterns)
    pub path_overrides: HashMap<String, PermissionLevel>,
    /// Whether to auto-approve trivial changes
    pub auto_approve_trivial: bool,
    /// Maximum lines changed for auto-approval
    pub trivial_threshold_lines: usize,
    /// Require tests to pass before commit
    pub require_tests: bool,
    /// Require lint to pass before commit
    pub require_lint: bool,
}

impl Default for ProjectPermissions {
    fn default() -> Self {
        Self {
            base_level: PermissionLevel::Supervised,
            path_overrides: HashMap::new(),
            auto_approve_trivial: true,
            trivial_threshold_lines: 50,
            require_tests: true,
            require_lint: true,
        }
    }
}

impl ProjectPermissions {
    /// Create permissions for Velofi (high security)
    pub fn velofi() -> Self {
        let mut path_overrides = HashMap::new();
        // Critical paths require explicit approval
        path_overrides.insert("**/auth/**".to_string(), PermissionLevel::Restricted);
        path_overrides.insert("**/payment*".to_string(), PermissionLevel::Restricted);
        path_overrides.insert("**/wallet*".to_string(), PermissionLevel::Restricted);
        path_overrides.insert("**/.env*".to_string(), PermissionLevel::Restricted);
        path_overrides.insert("**/secrets*".to_string(), PermissionLevel::Restricted);

        Self {
            base_level: PermissionLevel::Supervised,
            path_overrides,
            auto_approve_trivial: false, // Always require approval for Velofi
            trivial_threshold_lines: 20,
            require_tests: true,
            require_lint: true,
        }
    }

    /// Create permissions for ClaudeBot (autonomous allowed)
    pub fn claudebot() -> Self {
        Self {
            base_level: PermissionLevel::Autonomous,
            path_overrides: HashMap::new(),
            auto_approve_trivial: true,
            trivial_threshold_lines: 100,
            require_tests: true,
            require_lint: true,
        }
    }
}

/// Session-based permission escalation
#[derive(Debug, Clone)]
pub struct SessionPermissions {
    /// User ID
    user_id: i64,
    /// Current permission level (can be escalated)
    current_level: PermissionLevel,
    /// When the escalation expires
    escalation_expires: Option<Instant>,
    /// Escalation duration
    escalation_duration: Duration,
    /// Operations approved this session
    approved_operations: Vec<Operation>,
}

impl SessionPermissions {
    pub fn new(user_id: i64, base_level: PermissionLevel) -> Self {
        Self {
            user_id,
            current_level: base_level,
            escalation_expires: None,
            escalation_duration: Duration::from_secs(3600), // 1 hour default
            approved_operations: Vec::new(),
        }
    }

    /// Check if an operation is allowed
    pub fn is_allowed(&self, op: Operation) -> bool {
        let effective_level = self.effective_level();
        let required = op.required_level();

        match (effective_level, required) {
            (PermissionLevel::Autonomous, _) => true,
            (PermissionLevel::Supervised, PermissionLevel::Restricted) => true,
            (PermissionLevel::Supervised, PermissionLevel::Supervised) => true,
            (PermissionLevel::Restricted, PermissionLevel::Restricted) => true,
            _ => false,
        }
    }

    /// Get effective permission level (considering escalation)
    pub fn effective_level(&self) -> PermissionLevel {
        if let Some(expires) = self.escalation_expires {
            if Instant::now() < expires {
                return PermissionLevel::Autonomous;
            }
        }
        self.current_level
    }

    /// Escalate to autonomous mode
    pub fn escalate(&mut self, duration: Option<Duration>) {
        let dur = duration.unwrap_or(self.escalation_duration);
        self.escalation_expires = Some(Instant::now() + dur);
        tracing::info!(
            "User {} escalated to Autonomous mode for {:?}",
            self.user_id, dur
        );
    }

    /// Revoke escalation
    pub fn revoke(&mut self) {
        self.escalation_expires = None;
        tracing::info!("User {} escalation revoked", self.user_id);
    }

    /// Approve a specific operation for this session
    pub fn approve_operation(&mut self, op: Operation) {
        if !self.approved_operations.contains(&op) {
            self.approved_operations.push(op);
        }
    }

    /// Check if operation was explicitly approved
    pub fn is_operation_approved(&self, op: Operation) -> bool {
        self.approved_operations.contains(&op)
    }

    /// Time remaining on escalation
    pub fn escalation_remaining(&self) -> Option<Duration> {
        self.escalation_expires.map(|expires| {
            expires.saturating_duration_since(Instant::now())
        })
    }
}

/// Permission manager for all users
pub struct PermissionManager {
    /// Project permissions by path prefix
    projects: HashMap<String, ProjectPermissions>,
    /// Active sessions by user ID
    sessions: RwLock<HashMap<i64, SessionPermissions>>,
    /// Default permission level for unknown projects
    default_level: PermissionLevel,
}

impl PermissionManager {
    pub fn new() -> Self {
        let mut projects = HashMap::new();

        // Configure known projects
        projects.insert("velofi".to_string(), ProjectPermissions::velofi());
        projects.insert("claudebot".to_string(), ProjectPermissions::claudebot());

        Self {
            projects,
            sessions: RwLock::new(HashMap::new()),
            default_level: PermissionLevel::Supervised,
        }
    }

    /// Get or create session for user
    pub fn get_session(&self, user_id: i64, project: Option<&str>) -> SessionPermissions {
        let base_level = project
            .and_then(|p| self.projects.get(p))
            .map(|pp| pp.base_level)
            .unwrap_or(self.default_level);

        let sessions = self.sessions.read().unwrap();
        sessions.get(&user_id)
            .cloned()
            .unwrap_or_else(|| SessionPermissions::new(user_id, base_level))
    }

    /// Check if operation is allowed for user in project
    pub fn check_permission(
        &self,
        user_id: i64,
        project: Option<&str>,
        path: Option<&str>,
        operation: Operation,
    ) -> PermissionCheck {
        let session = self.get_session(user_id, project);

        // Check session-level permission
        if session.is_allowed(operation) {
            // Check path-specific overrides
            if let (Some(proj), Some(p)) = (project, path) {
                if let Some(proj_perms) = self.projects.get(proj) {
                    for (pattern, level) in &proj_perms.path_overrides {
                        if glob_match(pattern, p) {
                            if *level == PermissionLevel::Restricted &&
                               operation.required_level() != PermissionLevel::Restricted {
                                return PermissionCheck::Denied {
                                    reason: format!("Path {} requires explicit approval", p),
                                    can_escalate: true,
                                };
                            }
                        }
                    }
                }
            }

            return PermissionCheck::Allowed;
        }

        PermissionCheck::NeedsApproval {
            operation,
            risk_level: operation.risk_level(),
        }
    }

    /// Escalate user to autonomous mode
    pub fn escalate_user(&self, user_id: i64, duration: Option<Duration>) {
        let mut sessions = self.sessions.write().unwrap();
        let session = sessions.entry(user_id)
            .or_insert_with(|| SessionPermissions::new(user_id, self.default_level));
        session.escalate(duration);
    }

    /// Revoke user escalation
    pub fn revoke_user(&self, user_id: i64) {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(&user_id) {
            session.revoke();
        }
    }

    /// Get escalation status for user
    pub fn get_status(&self, user_id: i64) -> PermissionStatus {
        let sessions = self.sessions.read().unwrap();
        if let Some(session) = sessions.get(&user_id) {
            PermissionStatus {
                level: session.effective_level(),
                escalation_remaining: session.escalation_remaining(),
                approved_ops: session.approved_operations.len(),
            }
        } else {
            PermissionStatus {
                level: self.default_level,
                escalation_remaining: None,
                approved_ops: 0,
            }
        }
    }
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of permission check
#[derive(Debug, Clone)]
pub enum PermissionCheck {
    /// Operation is allowed
    Allowed,
    /// Operation needs user approval
    NeedsApproval {
        operation: Operation,
        risk_level: u8,
    },
    /// Operation is denied
    Denied {
        reason: String,
        can_escalate: bool,
    },
}

/// Current permission status
#[derive(Debug, Clone)]
pub struct PermissionStatus {
    pub level: PermissionLevel,
    pub escalation_remaining: Option<Duration>,
    pub approved_ops: usize,
}

/// Simple glob matching (supports * and **)
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.contains("**") {
        // ** matches any path segment
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');
            return (prefix.is_empty() || path.starts_with(prefix)) &&
                   (suffix.is_empty() || path.ends_with(suffix) || path.contains(&format!("/{}", suffix)));
        }
    }

    // Simple * matching
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

/// Change proposal for approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeProposal {
    pub id: String,
    pub description: String,
    pub files_changed: Vec<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub risk_assessment: RiskAssessment,
    pub llama_review: Option<String>,
    pub created_at: i64,
}

/// Llama-based risk assessment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub score: u8,           // 0-10
    pub category: RiskCategory,
    pub concerns: Vec<String>,
    pub recommendation: ApprovalRecommendation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskCategory {
    Trivial,      // Typos, comments, formatting
    Low,          // Small refactors, test additions
    Medium,       // New features, bug fixes
    High,         // Security-related, API changes
    Critical,     // Auth, payments, deployments
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalRecommendation {
    AutoApprove,   // Safe to auto-merge
    QuickReview,   // Glance review sufficient
    FullReview,    // Careful review needed
    ExpertReview,  // Security/domain expert needed
    Block,         // Do not merge without discussion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_levels() {
        let mut session = SessionPermissions::new(123, PermissionLevel::Supervised);

        assert!(session.is_allowed(Operation::Read));
        assert!(session.is_allowed(Operation::Write));
        assert!(!session.is_allowed(Operation::Push));

        session.escalate(Some(Duration::from_secs(60)));
        assert!(session.is_allowed(Operation::Push));
        assert!(session.is_allowed(Operation::Deploy));
    }

    #[test]
    fn test_glob_matching() {
        assert!(glob_match("**/auth/**", "src/auth/login.rs"));
        assert!(glob_match("**/auth/**", "crates/api/src/auth/middleware.rs"));
        assert!(!glob_match("**/auth/**", "src/main.rs"));

        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("src/*.rs", "src/lib.rs"));
    }

    #[test]
    fn test_project_permissions() {
        let velofi = ProjectPermissions::velofi();
        assert_eq!(velofi.base_level, PermissionLevel::Supervised);
        assert!(!velofi.auto_approve_trivial);

        let claudebot = ProjectPermissions::claudebot();
        assert_eq!(claudebot.base_level, PermissionLevel::Autonomous);
        assert!(claudebot.auto_approve_trivial);
    }
}
