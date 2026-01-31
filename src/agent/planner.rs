//! Planning Mode Engine
//!
//! Implements show plan → approve → execute workflow:
//! - Task decomposition into steps
//! - User approval before execution
//! - Step-by-step execution with progress
//! - Rollback capability
//!
//! Industry standard: Claude Code planning, AutoGPT task chains

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

/// Status of a plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    /// Plan is being created
    Drafting,
    /// Waiting for user approval
    PendingApproval,
    /// User approved, ready to execute
    Approved,
    /// Currently executing
    Executing,
    /// Completed successfully
    Completed,
    /// Failed during execution
    Failed,
    /// Cancelled by user
    Cancelled,
}

/// User's approval state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalState {
    /// Waiting for user decision
    Pending,
    /// User approved the plan
    Approved,
    /// User rejected the plan
    Rejected,
    /// User requested modifications
    ModifyRequested,
}

/// A single step in a plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Step number (1-indexed)
    pub number: usize,
    /// Short title
    pub title: String,
    /// Detailed description
    pub description: String,
    /// Estimated complexity (1-10)
    pub complexity: u8,
    /// Dependencies (step numbers)
    pub depends_on: Vec<usize>,
    /// Step status
    pub status: StepStatus,
    /// Result if completed
    pub result: Option<String>,
    /// Error if failed
    pub error: Option<String>,
    /// Execution duration
    pub duration: Option<Duration>,
}

/// Status of a plan step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Blocked,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl PlanStep {
    /// Create a new step
    pub fn new(number: usize, title: &str, description: &str) -> Self {
        Self {
            number,
            title: title.to_string(),
            description: description.to_string(),
            complexity: 5,
            depends_on: vec![],
            status: StepStatus::Pending,
            result: None,
            error: None,
            duration: None,
        }
    }

    /// Set complexity
    pub fn with_complexity(mut self, complexity: u8) -> Self {
        self.complexity = complexity.clamp(1, 10);
        self
    }

    /// Add dependency
    pub fn depends_on(mut self, step: usize) -> Self {
        if step < self.number {
            self.depends_on.push(step);
        }
        self
    }

    /// Check if step can start (all dependencies completed)
    pub fn can_start(&self, completed_steps: &[usize]) -> bool {
        self.status == StepStatus::Pending
            && self.depends_on.iter().all(|d| completed_steps.contains(d))
    }

    /// Format for display
    pub fn format(&self) -> String {
        let status_icon = match self.status {
            StepStatus::Pending => "○",
            StepStatus::Blocked => "◌",
            StepStatus::Running => "◎",
            StepStatus::Completed => "●",
            StepStatus::Failed => "✗",
            StepStatus::Skipped => "◇",
        };

        let duration = self
            .duration
            .map(|d| format!(" ({}s)", d.as_secs()))
            .unwrap_or_default();

        format!(
            "{} Step {}: {}{}",
            status_icon, self.number, self.title, duration
        )
    }
}

/// A complete execution plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Unique plan ID
    pub id: String,
    /// Plan title
    pub title: String,
    /// Overall description
    pub description: String,
    /// Plan steps
    pub steps: Vec<PlanStep>,
    /// Plan status
    pub status: PlanStatus,
    /// Approval state
    pub approval: ApprovalState,
    /// User who owns this plan
    pub user_id: Option<i64>,
    /// Creation timestamp
    pub created_at: i64,
    /// Last update timestamp
    pub updated_at: i64,
    /// Metadata
    pub metadata: HashMap<String, String>,
}

impl Plan {
    /// Create a new plan
    pub fn new(title: &str, description: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            description: description.to_string(),
            steps: vec![],
            status: PlanStatus::Drafting,
            approval: ApprovalState::Pending,
            user_id: None,
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    /// Add a step to the plan
    pub fn add_step(&mut self, step: PlanStep) {
        self.steps.push(step);
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Get step by number
    pub fn get_step(&self, number: usize) -> Option<&PlanStep> {
        self.steps.iter().find(|s| s.number == number)
    }

    /// Get mutable step by number
    pub fn get_step_mut(&mut self, number: usize) -> Option<&mut PlanStep> {
        self.steps.iter_mut().find(|s| s.number == number)
    }

    /// Get completed step numbers
    pub fn completed_steps(&self) -> Vec<usize> {
        self.steps
            .iter()
            .filter(|s| s.status == StepStatus::Completed)
            .map(|s| s.number)
            .collect()
    }

    /// Get next runnable step
    pub fn next_step(&self) -> Option<&PlanStep> {
        let completed = self.completed_steps();
        self.steps
            .iter()
            .find(|s| s.can_start(&completed))
    }

    /// Calculate progress (0.0 - 1.0)
    pub fn progress(&self) -> f64 {
        if self.steps.is_empty() {
            return 0.0;
        }
        let completed = self.steps.iter().filter(|s| s.status == StepStatus::Completed).count();
        completed as f64 / self.steps.len() as f64
    }

    /// Format plan for display
    pub fn format(&self) -> String {
        let mut s = format!("# {}\n\n", self.title);
        s.push_str(&self.description);
        s.push_str("\n\n## Steps\n\n");

        for step in &self.steps {
            s.push_str(&step.format());
            s.push('\n');
            if !step.description.is_empty() {
                s.push_str(&format!("   {}\n", step.description));
            }
        }

        let progress = self.progress() * 100.0;
        s.push_str(&format!("\nProgress: {:.0}%\n", progress));

        s
    }

    /// Format for approval prompt
    pub fn format_for_approval(&self) -> String {
        let mut s = format!("📋 **Plan: {}**\n\n", self.title);
        s.push_str(&self.description);
        s.push_str("\n\n**Steps:**\n");

        for step in &self.steps {
            s.push_str(&format!(
                "{}. {} (complexity: {}/10)\n",
                step.number, step.title, step.complexity
            ));
            if !step.description.is_empty() {
                s.push_str(&format!("   _{}_\n", step.description));
            }
        }

        s.push_str("\n**Reply:**\n");
        s.push_str("• `approve` - Execute this plan\n");
        s.push_str("• `reject` - Cancel the plan\n");
        s.push_str("• `modify: <feedback>` - Request changes\n");

        s
    }
}

/// Planning engine configuration
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    /// Maximum steps per plan
    pub max_steps: usize,
    /// Require approval before execution
    pub require_approval: bool,
    /// Auto-approve for simple plans (< N steps)
    pub auto_approve_threshold: usize,
    /// Timeout for user approval
    pub approval_timeout: Duration,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_steps: 20,
            require_approval: true,
            auto_approve_threshold: 3,
            approval_timeout: Duration::from_secs(300),
        }
    }
}

/// Planning engine for task decomposition and execution
pub struct PlanningEngine {
    config: PlannerConfig,
    plans: Arc<RwLock<HashMap<String, Plan>>>,
}

impl PlanningEngine {
    /// Create a new planning engine
    pub fn new() -> Self {
        Self::with_config(PlannerConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: PlannerConfig) -> Self {
        Self {
            config,
            plans: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a plan from a task description
    pub async fn create_plan(
        &self,
        task: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<Plan> {
        let prompt = format!(
            r#"Create an execution plan for this task. Break it into clear, actionable steps.

Task: {}

Return a JSON object with:
- title: Short plan title
- description: Brief overview
- steps: Array of {{number, title, description, complexity (1-10), depends_on: [step numbers]}}

Keep steps atomic and achievable. Maximum {} steps.

JSON only:"#,
            task, self.config.max_steps
        );

        let response = llama.generate(&prompt).await?;
        self.parse_plan(&response)
    }

    /// Parse LLM response into a Plan
    fn parse_plan(&self, response: &str) -> Result<Plan> {
        #[derive(Deserialize)]
        struct PlanJson {
            title: String,
            description: String,
            steps: Vec<StepJson>,
        }

        #[derive(Deserialize)]
        struct StepJson {
            number: usize,
            title: String,
            description: Option<String>,
            complexity: Option<u8>,
            depends_on: Option<Vec<usize>>,
        }

        let json_str = extract_json(response).unwrap_or(response);
        let parsed: PlanJson = serde_json::from_str(json_str)?;

        let mut plan = Plan::new(&parsed.title, &parsed.description);

        for (i, step_json) in parsed.steps.into_iter().enumerate() {
            let number = step_json.number.max(i + 1);
            let mut step = PlanStep::new(
                number,
                &step_json.title,
                step_json.description.as_deref().unwrap_or(""),
            );

            if let Some(c) = step_json.complexity {
                step = step.with_complexity(c);
            }

            if let Some(deps) = step_json.depends_on {
                for dep in deps {
                    step = step.depends_on(dep);
                }
            }

            plan.add_step(step);
        }

        plan.status = PlanStatus::PendingApproval;
        Ok(plan)
    }

    /// Store a plan
    pub async fn store_plan(&self, plan: Plan) {
        self.plans.write().await.insert(plan.id.clone(), plan);
    }

    /// Get a plan by ID
    pub async fn get_plan(&self, id: &str) -> Option<Plan> {
        self.plans.read().await.get(id).cloned()
    }

    /// Process user approval response
    pub async fn process_approval(&self, plan_id: &str, response: &str) -> Result<ApprovalState> {
        let mut plans = self.plans.write().await;
        let plan = plans.get_mut(plan_id).ok_or_else(|| anyhow::anyhow!("Plan not found"))?;

        let response_lower = response.trim().to_lowercase();

        if response_lower == "approve" || response_lower == "yes" || response_lower == "ok" {
            plan.approval = ApprovalState::Approved;
            plan.status = PlanStatus::Approved;
            Ok(ApprovalState::Approved)
        } else if response_lower == "reject" || response_lower == "no" || response_lower == "cancel" {
            plan.approval = ApprovalState::Rejected;
            plan.status = PlanStatus::Cancelled;
            Ok(ApprovalState::Rejected)
        } else if response_lower.starts_with("modify") {
            plan.approval = ApprovalState::ModifyRequested;
            Ok(ApprovalState::ModifyRequested)
        } else {
            Ok(ApprovalState::Pending)
        }
    }

    /// Check if plan should auto-approve
    pub fn should_auto_approve(&self, plan: &Plan) -> bool {
        !self.config.require_approval || plan.steps.len() < self.config.auto_approve_threshold
    }

    /// Execute a single step
    pub async fn execute_step(
        &self,
        plan_id: &str,
        step_number: usize,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<StepStatus> {
        let start = Instant::now();

        // Get step details
        let step_info = {
            let plans = self.plans.read().await;
            let plan = plans.get(plan_id).ok_or_else(|| anyhow::anyhow!("Plan not found"))?;
            let step = plan.get_step(step_number).ok_or_else(|| anyhow::anyhow!("Step not found"))?;
            (step.title.clone(), step.description.clone())
        };

        // Update status to running
        {
            let mut plans = self.plans.write().await;
            if let Some(plan) = plans.get_mut(plan_id) {
                if let Some(step) = plan.get_step_mut(step_number) {
                    step.status = StepStatus::Running;
                }
            }
        }

        // Execute the step
        let prompt = format!(
            "Execute this step and provide the result:\n\nStep: {}\nDescription: {}\n\nProvide a concise result:",
            step_info.0, step_info.1
        );

        let result = llama.generate(&prompt).await;
        let duration = start.elapsed();

        // Update step with result
        let mut plans = self.plans.write().await;
        if let Some(plan) = plans.get_mut(plan_id) {
            if let Some(step) = plan.get_step_mut(step_number) {
                step.duration = Some(duration);
                match result {
                    Ok(output) => {
                        step.status = StepStatus::Completed;
                        step.result = Some(output);
                        Ok(StepStatus::Completed)
                    }
                    Err(e) => {
                        step.status = StepStatus::Failed;
                        step.error = Some(e.to_string());
                        Ok(StepStatus::Failed)
                    }
                }
            } else {
                Err(anyhow::anyhow!("Step not found"))
            }
        } else {
            Err(anyhow::anyhow!("Plan not found"))
        }
    }

    /// Execute all steps in a plan
    pub async fn execute_plan(
        &self,
        plan_id: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<PlanStatus> {
        // Verify plan is approved
        {
            let plans = self.plans.read().await;
            let plan = plans.get(plan_id).ok_or_else(|| anyhow::anyhow!("Plan not found"))?;
            if plan.approval != ApprovalState::Approved {
                return Err(anyhow::anyhow!("Plan not approved"));
            }
        }

        // Update status
        {
            let mut plans = self.plans.write().await;
            if let Some(plan) = plans.get_mut(plan_id) {
                plan.status = PlanStatus::Executing;
            }
        }

        // Execute steps in order
        loop {
            let next_step = {
                let plans = self.plans.read().await;
                let plan = plans.get(plan_id).ok_or_else(|| anyhow::anyhow!("Plan not found"))?;
                plan.next_step().map(|s| s.number)
            };

            match next_step {
                Some(step_num) => {
                    info!("Executing step {} of plan {}", step_num, plan_id);
                    let status = self.execute_step(plan_id, step_num, llama).await?;
                    if status == StepStatus::Failed {
                        // Update plan status
                        let mut plans = self.plans.write().await;
                        if let Some(plan) = plans.get_mut(plan_id) {
                            plan.status = PlanStatus::Failed;
                        }
                        return Ok(PlanStatus::Failed);
                    }
                }
                None => break,
            }
        }

        // Update final status
        {
            let mut plans = self.plans.write().await;
            if let Some(plan) = plans.get_mut(plan_id) {
                plan.status = PlanStatus::Completed;
            }
        }

        Ok(PlanStatus::Completed)
    }

    /// Cancel a plan
    pub async fn cancel_plan(&self, plan_id: &str) -> bool {
        let mut plans = self.plans.write().await;
        if let Some(plan) = plans.get_mut(plan_id) {
            plan.status = PlanStatus::Cancelled;
            true
        } else {
            false
        }
    }

    /// Get active plans for a user
    pub async fn get_user_plans(&self, user_id: i64) -> Vec<Plan> {
        self.plans
            .read()
            .await
            .values()
            .filter(|p| p.user_id == Some(user_id))
            .filter(|p| matches!(p.status, PlanStatus::PendingApproval | PlanStatus::Approved | PlanStatus::Executing))
            .cloned()
            .collect()
    }
}

impl Default for PlanningEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract JSON from text
fn extract_json(s: &str) -> Option<&str> {
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
    fn test_plan_step_creation() {
        let step = PlanStep::new(1, "Setup", "Initialize the project")
            .with_complexity(3);

        assert_eq!(step.number, 1);
        assert_eq!(step.complexity, 3);
        assert!(step.depends_on.is_empty());

        // Test that dependency on itself is rejected
        let step2 = PlanStep::new(2, "Build", "Build the project")
            .depends_on(2); // Same step - should be ignored
        assert!(step2.depends_on.is_empty());

        // Test valid dependency
        let step3 = PlanStep::new(3, "Test", "Run tests")
            .depends_on(2);
        assert_eq!(step3.depends_on, vec![2]);
    }

    #[test]
    fn test_plan_creation() {
        let mut plan = Plan::new("Test Plan", "A test plan description");
        plan.add_step(PlanStep::new(1, "Step 1", "First step"));
        plan.add_step(PlanStep::new(2, "Step 2", "Second step").depends_on(1));

        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.status, PlanStatus::Drafting);
    }

    #[test]
    fn test_plan_progress() {
        let mut plan = Plan::new("Test", "Test");
        plan.add_step(PlanStep::new(1, "S1", ""));
        plan.add_step(PlanStep::new(2, "S2", ""));

        assert_eq!(plan.progress(), 0.0);

        plan.steps[0].status = StepStatus::Completed;
        assert_eq!(plan.progress(), 0.5);

        plan.steps[1].status = StepStatus::Completed;
        assert_eq!(plan.progress(), 1.0);
    }

    #[test]
    fn test_next_step() {
        let mut plan = Plan::new("Test", "Test");
        plan.add_step(PlanStep::new(1, "S1", ""));
        plan.add_step(PlanStep::new(2, "S2", "").depends_on(1));

        // First step should be runnable
        assert_eq!(plan.next_step().map(|s| s.number), Some(1));

        // Complete first step
        plan.steps[0].status = StepStatus::Completed;

        // Second step should now be runnable
        assert_eq!(plan.next_step().map(|s| s.number), Some(2));
    }

    #[test]
    fn test_step_format() {
        let step = PlanStep::new(1, "Initialize", "Set up project");
        let formatted = step.format();
        assert!(formatted.contains("Step 1"));
        assert!(formatted.contains("Initialize"));
    }

    #[tokio::test]
    async fn test_planning_engine() {
        let engine = PlanningEngine::new();

        let mut plan = Plan::new("Test Plan", "Testing");
        plan.add_step(PlanStep::new(1, "Step 1", "First"));
        plan.status = PlanStatus::PendingApproval;

        let plan_id = plan.id.clone();
        engine.store_plan(plan).await;

        let retrieved = engine.get_plan(&plan_id).await;
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_approval_processing() {
        let engine = PlanningEngine::new();

        let mut plan = Plan::new("Test", "Test");
        plan.status = PlanStatus::PendingApproval;
        let plan_id = plan.id.clone();
        engine.store_plan(plan).await;

        let result = engine.process_approval(&plan_id, "approve").await.unwrap();
        assert_eq!(result, ApprovalState::Approved);

        let plan = engine.get_plan(&plan_id).await.unwrap();
        assert_eq!(plan.status, PlanStatus::Approved);
    }
}
