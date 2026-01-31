//! Ultimate AI Agent System
//!
//! A world-class autonomous agent framework implementing:
//! - Reflection & Self-Correction (Constitutional AI patterns)
//! - Multi-Agent Orchestration (specialized sub-agents)
//! - Structured Tool Use (JSON schema tools with parallel execution)
//! - Planning Mode (show plan → approve → execute)
//! - Streaming Responses (real-time output)
//! - Proactive Notifications (scheduled reminders)
//! - Error Recovery (retry with exponential backoff)
//!
//! Architecture follows the OODA loop enhanced with reflection:
//! Observe → Orient → Decide → Act → Reflect → Learn

pub mod reflection;
pub mod orchestrator;
pub mod tools;
pub mod planner;
pub mod streaming;
pub mod scheduler;
pub mod recovery;

pub use reflection::{ReflectionEngine, ReflectionResult, QualityScore};
pub use orchestrator::{AgentOrchestrator, SubAgent, AgentTask, AgentResult};
pub use tools::{ToolRegistry, Tool, ToolCall, ToolResult, ToolSchema};
pub use planner::{PlanningEngine, Plan, PlanStep, PlanStatus, ApprovalState};
pub use streaming::{StreamingResponse, StreamChunk, StreamHandle};
pub use scheduler::{Scheduler, ScheduledTask, Reminder, NotificationType};
pub use recovery::{RecoveryStrategy, RetryPolicy, CircuitBreaker, RecoveryAction};
