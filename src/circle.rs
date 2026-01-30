//! Development Circle (E5)
//!
//! Multi-persona code quality pipeline with 5 phases:
//! 1. Carmack - Implementation (John Carmack style - elegant, efficient, correct)
//! 2. Linus - Code Review (Linus Torvalds rigor)
//! 3. Maria - Testing (Kent Beck TDD mastery)
//! 4. Kai - Optimization (Data-oriented design)
//! 5. Sentinel - Security Audit (OWASP + breach mentality)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::claude::ClaudeClient;

/// Maximum number of revision attempts
const MAX_REVISIONS: u32 = 3;

/// Pipeline execution mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineMode {
    /// Full 5-phase pipeline
    #[default]
    Full,
    /// Review only (Linus + Sentinel)
    ReviewOnly,
    /// Quick fix (Carmack only)
    QuickFix,
    /// Security audit (Sentinel only)
    SecurityOnly,
}

/// Persona in the development circle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Persona {
    /// Legendary Engineer - Implementation (John Carmack style)
    Carmack,
    /// Tech Lead - Code Review (Linus Torvalds rigor)
    Linus,
    /// QA Mastermind - Testing (Kent Beck TDD)
    Maria,
    /// Performance Craftsman - Optimization (Data-oriented design)
    Kai,
    /// Security Guardian - Audit (OWASP + breach mentality)
    Sentinel,
}

impl Persona {
    pub fn name(&self) -> &'static str {
        match self {
            Persona::Carmack => "Carmack",
            Persona::Linus => "Linus",
            Persona::Maria => "Maria",
            Persona::Kai => "Kai",
            Persona::Sentinel => "Sentinel",
        }
    }

    pub fn role(&self) -> &'static str {
        match self {
            Persona::Carmack => "Implementation",
            Persona::Linus => "Code Review",
            Persona::Maria => "Testing",
            Persona::Kai => "Optimization",
            Persona::Sentinel => "Security Audit",
        }
    }

    pub fn phase(&self) -> u8 {
        match self {
            Persona::Carmack => 1,
            Persona::Linus => 2,
            Persona::Maria => 3,
            Persona::Kai => 4,
            Persona::Sentinel => 5,
        }
    }

    /// Get the persona's system prompt
    pub fn system_prompt(&self) -> &'static str {
        match self {
            Persona::Carmack => CARMACK_PROMPT,
            Persona::Linus => LINUS_PROMPT,
            Persona::Maria => MARIA_PROMPT,
            Persona::Kai => KAI_PROMPT,
            Persona::Sentinel => SENTINEL_PROMPT,
        }
    }

    /// Model hint for this persona
    pub fn model_hint(&self) -> &'static str {
        match self {
            Persona::Carmack => "sonnet",
            Persona::Linus => "sonnet",
            Persona::Maria => "sonnet",
            Persona::Kai => "sonnet",
            Persona::Sentinel => "opus", // Security needs deep reasoning
        }
    }
}

/// Review verdict from Linus or Sentinel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Approved,
    ApprovedWithComments,
    ChangesRequested,
    Blocked,
}

impl Verdict {
    pub fn is_approved(&self) -> bool {
        matches!(self, Verdict::Approved | Verdict::ApprovedWithComments)
    }
}

/// Security risk level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn is_acceptable(&self) -> bool {
        matches!(self, RiskLevel::Low | RiskLevel::Medium)
    }
}

/// Result of a single phase execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    pub persona: String,
    pub phase: u8,
    pub output: String,
    pub verdict: Option<Verdict>,
    pub risk_level: Option<RiskLevel>,
    pub files_changed: Vec<String>,
    pub duration_ms: u64,
}

/// Overall pipeline result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub feature: String,
    pub mode: PipelineMode,
    pub phases: Vec<PhaseResult>,
    pub revisions: u32,
    pub success: bool,
    pub blocked_at: Option<String>,
    pub total_duration_ms: u64,
}

/// Pipeline execution state
#[derive(Debug, Clone)]
pub struct PipelineState {
    pub feature: String,
    pub mode: PipelineMode,
    pub current_phase: u8,
    pub revision: u32,
    pub phases: Vec<PhaseResult>,
    pub code_context: String,
    pub feedback: Option<String>,
}

/// Development Circle orchestrator
pub struct Circle {
    claude: ClaudeClient,
}

impl Circle {
    /// Create new circle with Claude client
    pub fn new(claude: ClaudeClient) -> Self {
        Self { claude }
    }

    /// Run the full development circle pipeline
    pub async fn run(
        &self,
        feature: &str,
        context: &str,
        mode: PipelineMode,
    ) -> Result<PipelineResult> {
        let start = std::time::Instant::now();
        info!("Starting Development Circle: {} (mode: {:?})", feature, mode);

        let mut state = PipelineState {
            feature: feature.to_string(),
            mode,
            current_phase: 1,
            revision: 0,
            phases: Vec::new(),
            code_context: context.to_string(),
            feedback: None,
        };

        let phases = match mode {
            PipelineMode::Full => vec![
                Persona::Carmack,
                Persona::Linus,
                Persona::Maria,
                Persona::Kai,
                Persona::Sentinel,
            ],
            PipelineMode::ReviewOnly => vec![Persona::Linus, Persona::Sentinel],
            PipelineMode::QuickFix => vec![Persona::Carmack],
            PipelineMode::SecurityOnly => vec![Persona::Sentinel],
        };

        let mut phase_idx = 0;

        while phase_idx < phases.len() {
            let persona = phases[phase_idx];
            state.current_phase = persona.phase();

            let result = self.execute_phase(&state, persona).await?;

            // Handle review verdicts
            if persona == Persona::Linus {
                if let Some(verdict) = &result.verdict {
                    if *verdict == Verdict::ChangesRequested {
                        state.revision += 1;
                        if state.revision > MAX_REVISIONS {
                            warn!("Max revisions ({}) exceeded", MAX_REVISIONS);
                            state.phases.push(result);
                            return Ok(PipelineResult {
                                feature: state.feature,
                                mode: state.mode,
                                phases: state.phases,
                                revisions: state.revision,
                                success: false,
                                blocked_at: Some("Linus - Max Revisions".to_string()),
                                total_duration_ms: start.elapsed().as_millis() as u64,
                            });
                        }

                        // Return to Carmack with feedback
                        state.feedback = Some(result.output.clone());
                        state.phases.push(result);
                        phase_idx = 0; // Back to Carmack
                        continue;
                    }
                }
            }

            // Handle security verdicts
            if persona == Persona::Sentinel {
                if let Some(risk) = result.risk_level {
                    if !risk.is_acceptable() {
                        warn!("Security blocked: {:?}", risk);
                        let blocked_reason = format!("Sentinel - {:?} Risk", risk);
                        state.phases.push(result);
                        return Ok(PipelineResult {
                            feature: state.feature,
                            mode: state.mode,
                            phases: state.phases,
                            revisions: state.revision,
                            success: false,
                            blocked_at: Some(blocked_reason),
                            total_duration_ms: start.elapsed().as_millis() as u64,
                        });
                    }
                }
            }

            // Update code context for next phase
            if !result.output.is_empty() && persona == Persona::Carmack {
                state.code_context = result.output.clone();
            }

            state.phases.push(result);
            state.feedback = None;
            phase_idx += 1;
        }

        info!("Development Circle complete: {} phases", state.phases.len());

        Ok(PipelineResult {
            feature: state.feature,
            mode: state.mode,
            phases: state.phases,
            revisions: state.revision,
            success: true,
            blocked_at: None,
            total_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Execute a single phase with the given persona
    async fn execute_phase(&self, state: &PipelineState, persona: Persona) -> Result<PhaseResult> {
        let start = std::time::Instant::now();
        debug!(
            "[{}/5] {} - {}",
            persona.phase(),
            persona.name(),
            persona.role()
        );

        let prompt = self.build_prompt(state, persona);
        let system = persona.system_prompt();
        let model = persona.model_hint();

        let response = self
            .claude
            .complete(&prompt, system, None, 8192, model)
            .await?;

        // Parse verdict from response (for Linus)
        let verdict = if persona == Persona::Linus {
            Self::parse_verdict(&response.content)
        } else {
            None
        };

        // Parse risk level from response (for Sentinel)
        let risk_level = if persona == Persona::Sentinel {
            Self::parse_risk_level(&response.content)
        } else {
            None
        };

        // Extract mentioned files
        let files_changed = Self::extract_files(&response.content);

        Ok(PhaseResult {
            persona: persona.name().to_string(),
            phase: persona.phase(),
            output: response.content,
            verdict,
            risk_level,
            files_changed,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Build the prompt for a phase
    fn build_prompt(&self, state: &PipelineState, persona: Persona) -> String {
        let mut prompt = format!(
            "## Feature Request\n\n{}\n\n## Current Code Context\n\n{}",
            state.feature, state.code_context
        );

        // Add feedback for revisions
        if let Some(feedback) = &state.feedback {
            prompt.push_str(&format!(
                "\n\n## Revision {} - Previous Feedback\n\n{}",
                state.revision, feedback
            ));
        }

        // Add phase-specific instructions
        match persona {
            Persona::Carmack => {
                prompt.push_str("\n\n## Task\n\nImplement this feature completely. Create all necessary files and write production-ready code.");
            }
            Persona::Linus => {
                prompt.push_str("\n\n## Task\n\nReview this implementation. End your review with exactly one of: APPROVED, APPROVED_WITH_COMMENTS, CHANGES_REQUESTED, BLOCKED");
            }
            Persona::Maria => {
                prompt.push_str("\n\n## Task\n\nWrite comprehensive tests for this implementation. Cover happy paths, edge cases, and error conditions.");
            }
            Persona::Kai => {
                prompt.push_str("\n\n## Task\n\nOptimize this code for performance. Focus on allocations, hot paths, and code elegance.");
            }
            Persona::Sentinel => {
                prompt.push_str("\n\n## Task\n\nPerform a security audit. Check OWASP Top 10. End with risk level: LOW, MEDIUM, HIGH, or CRITICAL");
            }
        }

        prompt
    }

    /// Parse verdict from Linus review
    fn parse_verdict(response: &str) -> Option<Verdict> {
        let upper = response.to_uppercase();
        if upper.contains("CHANGES_REQUESTED") || upper.contains("CHANGES REQUESTED") {
            Some(Verdict::ChangesRequested)
        } else if upper.contains("BLOCKED") {
            Some(Verdict::Blocked)
        } else if upper.contains("APPROVED_WITH_COMMENTS") || upper.contains("APPROVED WITH COMMENTS") {
            Some(Verdict::ApprovedWithComments)
        } else if upper.contains("APPROVED") {
            Some(Verdict::Approved)
        } else {
            None
        }
    }

    /// Parse risk level from Sentinel audit
    fn parse_risk_level(response: &str) -> Option<RiskLevel> {
        let upper = response.to_uppercase();
        if upper.contains("CRITICAL") {
            Some(RiskLevel::Critical)
        } else if upper.contains("HIGH") && upper.contains("RISK") {
            Some(RiskLevel::High)
        } else if upper.contains("MEDIUM") && upper.contains("RISK") {
            Some(RiskLevel::Medium)
        } else {
            // Default to LOW if "LOW RISK" mentioned or not specified
            Some(RiskLevel::Low)
        }
    }

    /// Extract file paths mentioned in response
    fn extract_files(response: &str) -> Vec<String> {
        let file_regex = regex::Regex::new(r"`([a-zA-Z0-9_/.-]+\.(rs|ts|vue|md|toml|json))`").ok();

        if let Some(re) = file_regex {
            re.captures_iter(response)
                .map(|c| c[1].to_string())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get pipeline status summary
    pub fn summarize(result: &PipelineResult) -> String {
        let mut summary = String::new();

        summary.push_str(&format!("# Development Circle: {}\n\n", result.feature));
        summary.push_str(&format!("**Mode:** {:?}\n", result.mode));
        summary.push_str(&format!("**Revisions:** {}\n", result.revisions));
        summary.push_str(&format!("**Success:** {}\n", result.success));

        if let Some(blocked) = &result.blocked_at {
            summary.push_str(&format!("**Blocked At:** {}\n", blocked));
        }

        summary.push_str(&format!(
            "**Duration:** {}ms\n\n",
            result.total_duration_ms
        ));

        summary.push_str("## Phases\n\n");
        for phase in &result.phases {
            summary.push_str(&format!(
                "- [{}] {} ({:?}): {}ms\n",
                phase.phase,
                phase.persona,
                phase.verdict.as_ref().map(|v| format!("{:?}", v)).unwrap_or_default(),
                phase.duration_ms
            ));
        }

        summary
    }
}

// ============================================================================
// PERSONA SYSTEM PROMPTS
// ============================================================================

const CARMACK_PROMPT: &str = r#"You are Carmack, a legendary Implementation Engineer channeling:
- John Carmack (id Software) - optimization genius, clean architecture, correctness first
- Rob Pike (Go, Plan 9) - simplicity, clarity, "less is more"
- Bryan Cantrill (DTrace, Oxide) - systems thinking, debugging mastery

"If you want to make something, make it well." - John Carmack

## Implementation Philosophy

1. Understand the problem DEEPLY before writing code
2. Design data structures first - they define the algorithm
3. Write the simplest solution that could possibly work
4. Handle ALL error cases explicitly - no shortcuts
5. Optimize only what measurements prove is slow

## Chain-of-Thought Process

Before implementation, think through:
1. What is the core abstraction/data structure?
2. What are ALL the failure modes?
3. What are the edge cases (zero, MAX, empty, concurrent)?
4. What would make this code obviously correct?

## Constraints

### Forbidden
- .unwrap() without safety proof - use .expect("reason") or ?
- Magic numbers - define constants with meaningful names
- f64 for money/financial - use Decimal
- panic! in library code - return Result<T, E>
- TODO comments - implement completely or don't commit
- Premature abstraction - earn complexity through need

### Required
- Result<T, E> for all fallible operations
- Meaningful error types with context
- /// doc comments on all public APIs
- Unit tests for core logic
- Zero compiler warnings

## Output Format

Provide complete, production-ready code:
1. Brief analysis of approach (3-5 sentences)
2. Complete implementation with file paths
3. Unit tests covering happy path and edge cases
4. Usage examples

Your code should compile cleanly and be obviously correct at first reading.
"#;

const LINUS_PROMPT: &str = r#"You are Linus, a Tech Lead performing code review.
Inspired by Linus Torvalds - direct, technically rigorous, constructive.

## Review Criteria

1. **Correctness** - Does it work? Edge cases handled?
2. **Readability** - Clear naming? Good structure?
3. **Performance** - Any obvious bottlenecks?
4. **Safety** - Proper error handling? No panics?
5. **Style** - Follows project conventions?

## Verdict Options

End your review with exactly ONE of:
- APPROVED - Ready to merge
- APPROVED_WITH_COMMENTS - Minor suggestions, can merge
- CHANGES_REQUESTED - Issues must be fixed
- BLOCKED - Critical problems, needs redesign

## Output Format

List specific issues with file:line references.
Be constructive - explain why and suggest fixes.
"#;

const MARIA_PROMPT: &str = r#"You are Maria, a QA Engineer specializing in test coverage.

## Testing Strategy

1. **Happy Path** - Normal successful operations
2. **Edge Cases** - Zero, MAX, empty, unicode, boundaries
3. **Error Paths** - Invalid input, failures, timeouts
4. **Integration** - Component interactions

## Velofi-Specific Tests

- Financial: Zero amounts, negative (should fail), precision loss
- Tax: 365/366 day boundary, year transitions
- HFT: Microsecond precision, concurrent access

## Output Format

Provide complete test files with:
- Descriptive test names (test_should_X_when_Y)
- Clear assertions
- Comments for non-obvious cases
"#;

const KAI_PROMPT: &str = r#"You are Kai, a Performance Engineer and Code Craftsman.

## Optimization Focus

1. **Allocations** - Reduce heap allocations in hot paths
2. **Copies** - Use references where possible
3. **Collections** - Right-size, prefer stack (SmallVec)
4. **Loops** - Iterator methods over manual loops
5. **Async** - Proper futures, avoid blocking

## Code Craft

- DRY without premature abstraction
- Clear naming over comments
- Small, focused functions
- Consistent style

## Output Format

Show before/after with metrics when possible.
Explain the performance impact of changes.
"#;

const SENTINEL_PROMPT: &str = r#"You are Sentinel, a Security Expert performing security audit.

## Audit Checklist (OWASP Top 10)

1. **Injection** - SQL, command, code injection vectors
2. **Auth** - Proper authentication, session management
3. **Data Exposure** - Sensitive data encrypted, logged?
4. **XXE** - XML parsing vulnerabilities
5. **Access Control** - Authorization checks present?
6. **Misconfiguration** - Secure defaults?
7. **XSS** - Output encoding?
8. **Deserialization** - Safe parsing?
9. **Components** - Known vulnerable dependencies?
10. **Logging** - Audit trails, no sensitive data logged?

## Velofi-Specific

- API keys never in code
- Balances/amounts validated
- Rate limiting on sensitive endpoints
- GDPR compliance for personal data

## Risk Levels

- LOW - Minor issues, acceptable for deployment
- MEDIUM - Should fix soon, can deploy with monitoring
- HIGH - Must fix before production
- CRITICAL - Immediate security threat, block deployment

## Output Format

List findings with severity, explanation, and remediation.
End with overall risk assessment.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verdict_parsing() {
        assert_eq!(
            Circle::parse_verdict("LGTM! APPROVED"),
            Some(Verdict::Approved)
        );
        assert_eq!(
            Circle::parse_verdict("APPROVED_WITH_COMMENTS - minor nits"),
            Some(Verdict::ApprovedWithComments)
        );
        assert_eq!(
            Circle::parse_verdict("CHANGES_REQUESTED: fix the error handling"),
            Some(Verdict::ChangesRequested)
        );
        assert_eq!(
            Circle::parse_verdict("This is BLOCKED due to design issues"),
            Some(Verdict::Blocked)
        );
    }

    #[test]
    fn test_risk_level_parsing() {
        assert_eq!(
            Circle::parse_risk_level("Risk Level: LOW"),
            Some(RiskLevel::Low)
        );
        assert_eq!(
            Circle::parse_risk_level("MEDIUM RISK - needs attention"),
            Some(RiskLevel::Medium)
        );
        assert_eq!(
            Circle::parse_risk_level("HIGH RISK: SQL injection possible"),
            Some(RiskLevel::High)
        );
        assert_eq!(
            Circle::parse_risk_level("CRITICAL security vulnerability"),
            Some(RiskLevel::Critical)
        );
    }

    #[test]
    fn test_file_extraction() {
        let response = "Modified `src/main.rs` and created `src/lib.rs`. Also updated `Cargo.toml`.";
        let files = Circle::extract_files(response);
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&"src/lib.rs".to_string()));
        assert!(files.contains(&"Cargo.toml".to_string()));
    }

    #[test]
    fn test_file_extraction_empty() {
        let response = "No files were modified.";
        let files = Circle::extract_files(response);
        assert!(files.is_empty());
    }

    #[test]
    fn test_file_extraction_duplicates() {
        let response = "`src/main.rs` and `src/main.rs` again `src/main.rs`";
        let files = Circle::extract_files(response);
        // Should deduplicate
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_persona_properties() {
        assert_eq!(Persona::Carmack.phase(), 1);
        assert_eq!(Persona::Sentinel.phase(), 5);
        assert_eq!(Persona::Sentinel.model_hint(), "opus");
        assert_eq!(Persona::Carmack.model_hint(), "sonnet");
    }

    #[test]
    fn test_all_personas_have_prompts() {
        let personas = [
            Persona::Carmack,
            Persona::Linus,
            Persona::Maria,
            Persona::Kai,
            Persona::Sentinel,
        ];

        for persona in personas {
            let prompt = persona.system_prompt();
            assert!(!prompt.is_empty());
            assert!(prompt.len() > 100); // Should be substantial
        }
    }

    #[test]
    fn test_verdict_edge_cases() {
        // Mixed case
        assert_eq!(
            Circle::parse_verdict("Approved"),
            Some(Verdict::Approved)
        );

        // With extra text
        assert_eq!(
            Circle::parse_verdict("After review: APPROVED - good work!"),
            Some(Verdict::Approved)
        );

        // No verdict
        assert_eq!(Circle::parse_verdict("Some random text"), None);
    }

    #[test]
    fn test_risk_level_acceptable() {
        assert!(RiskLevel::Low.is_acceptable());
        assert!(RiskLevel::Medium.is_acceptable());
        assert!(!RiskLevel::High.is_acceptable());
        assert!(!RiskLevel::Critical.is_acceptable());
    }

    #[test]
    fn test_pipeline_mode_default() {
        assert_eq!(PipelineMode::default(), PipelineMode::Full);
    }
}
