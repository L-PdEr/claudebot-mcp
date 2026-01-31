//! Autonomous Behavior System
//!
//! Implements industry-standard autonomous bot behavior:
//! - Auto-learning from conversations
//! - Proactive memory integration
//! - Background processing (consolidation, cleanup)
//! - Context continuity across sessions
//! - Self-improvement through feedback loops
//!
//! Architecture follows the OODA loop (Observe-Orient-Decide-Act):
//! 1. **Observe**: Extract facts, entities, and intents from messages
//! 2. **Orient**: Retrieve relevant context from memory and graph
//! 3. **Decide**: Route to appropriate model/action based on complexity
//! 4. **Act**: Execute and learn from the interaction

mod learner;
mod context_manager;
mod background;
mod goals;
mod feedback_loop;

pub use learner::{AutonomousLearner, LearnedFact, LearningConfig};
pub use context_manager::{ContextManager, EnrichedContext, ContextConfig};
pub use background::{BackgroundProcessor, BackgroundConfig, BackgroundTask};
pub use goals::{GoalTracker, Goal, GoalStatus};
pub use feedback_loop::{FeedbackLoop, FeedbackSignal, MemoryFeedback};
