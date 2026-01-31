//! Self-Extending Skills System
//!
//! Enables the agent to generate, install, and manage its own tools/skills.
//!
//! # Architecture
//!
//! ```text
//! User Request → Skill Detector → Skill Generator → Skill Registry
//!                     ↓                   ↓
//!               "Need new skill"    Claude generates
//!                     ↓                   ↓
//!               Store as goal      Parse & validate
//!                                        ↓
//!                                  Register tool
//!                                        ↓
//!                                  Persist to disk
//! ```
//!
//! # Skill Format
//!
//! Skills are defined in TOML format with embedded code:
//!
//! ```toml
//! [skill]
//! name = "weather"
//! version = "1.0.0"
//! description = "Get current weather for a location"
//!
//! [parameters]
//! location = { type = "string", description = "City name", required = true }
//!
//! [execution]
//! type = "http"  # or "shell", "script", "claude"
//! endpoint = "https://api.weather.com/v1/current"
//! method = "GET"
//! ```
//!
//! # Security
//!
//! Shell and script execution is sandboxed with:
//! - Command allowlist/blocklist
//! - Resource limits (timeout, memory, output size)
//! - Environment sanitization
//! - Pattern-based blocking for dangerous operations

pub mod registry;
pub mod generator;
pub mod loader;
pub mod types;
pub mod sandbox;

pub use registry::{SkillRegistry, InstalledSkill, SkillSource, SkillResult, SkillStats};
pub use generator::{SkillGenerator, GeneratedSkill};
pub use loader::SkillLoader;
pub use types::{SkillDefinition, SkillParameter, ExecutionType, SkillMetadata};
pub use sandbox::{SkillSandbox, SandboxConfig, SandboxResult, ValidationResult};
