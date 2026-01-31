//! Reflection & Self-Correction Engine
//!
//! Implements Constitutional AI patterns for self-improvement:
//! - Response quality evaluation (LLM-as-judge)
//! - Automatic retry on low quality
//! - Critique generation and response revision
//! - Multi-dimensional scoring (accuracy, helpfulness, safety)
//!
//! Industry standard: Anthropic's Constitutional AI, OpenAI's RLHF

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Quality dimensions for evaluation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityDimension {
    /// Is the response accurate and factual?
    Accuracy,
    /// Does it actually help the user?
    Helpfulness,
    /// Is it safe and appropriate?
    Safety,
    /// Is it well-structured and clear?
    Clarity,
    /// Does it follow instructions?
    Instruction,
    /// Is it complete (not truncated)?
    Completeness,
}

impl QualityDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accuracy => "accuracy",
            Self::Helpfulness => "helpfulness",
            Self::Safety => "safety",
            Self::Clarity => "clarity",
            Self::Instruction => "instruction_following",
            Self::Completeness => "completeness",
        }
    }

    pub fn weight(&self) -> f64 {
        match self {
            Self::Accuracy => 0.25,
            Self::Helpfulness => 0.25,
            Self::Safety => 0.20,
            Self::Clarity => 0.10,
            Self::Instruction => 0.15,
            Self::Completeness => 0.05,
        }
    }
}

/// Score for a single quality dimension (0.0 - 1.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScore {
    pub dimension: QualityDimension,
    pub score: f64,
    pub critique: Option<String>,
}

/// Overall quality score with breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    /// Weighted average score (0.0 - 1.0)
    pub overall: f64,
    /// Individual dimension scores
    pub dimensions: Vec<DimensionScore>,
    /// Generated critique explaining the scores
    pub critique: String,
    /// Suggested improvements
    pub improvements: Vec<String>,
    /// Should this response be retried?
    pub should_retry: bool,
}

impl QualityScore {
    /// Create a perfect score (for simple/command responses)
    pub fn perfect() -> Self {
        Self {
            overall: 1.0,
            dimensions: vec![],
            critique: "Response meets all criteria.".to_string(),
            improvements: vec![],
            should_retry: false,
        }
    }

    /// Create score from dimension scores
    pub fn from_dimensions(dimensions: Vec<DimensionScore>, critique: String) -> Self {
        let overall: f64 = dimensions
            .iter()
            .map(|d| d.score * d.dimension.weight())
            .sum::<f64>()
            / dimensions.iter().map(|d| d.dimension.weight()).sum::<f64>();

        let improvements: Vec<String> = dimensions
            .iter()
            .filter(|d| d.score < 0.7)
            .filter_map(|d| d.critique.clone())
            .collect();

        let should_retry = overall < 0.6 || dimensions.iter().any(|d| d.score < 0.4);

        Self {
            overall,
            dimensions,
            critique,
            improvements,
            should_retry,
        }
    }

    /// Format for display
    pub fn format(&self) -> String {
        let mut s = format!("Quality: {:.0}%\n", self.overall * 100.0);

        for dim in &self.dimensions {
            let icon = if dim.score >= 0.8 {
                "✓"
            } else if dim.score >= 0.6 {
                "○"
            } else {
                "✗"
            };
            s.push_str(&format!(
                "  {} {}: {:.0}%\n",
                icon,
                dim.dimension.as_str(),
                dim.score * 100.0
            ));
        }

        if !self.improvements.is_empty() {
            s.push_str("\nImprovements needed:\n");
            for imp in &self.improvements {
                s.push_str(&format!("  • {}\n", imp));
            }
        }

        s
    }
}

/// Result of reflection process
#[derive(Debug, Clone)]
pub struct ReflectionResult {
    /// Original response
    pub original: String,
    /// Quality evaluation
    pub quality: QualityScore,
    /// Revised response (if improvement was needed)
    pub revised: Option<String>,
    /// Number of revision attempts
    pub revision_count: u32,
    /// Final response to use
    pub final_response: String,
}

/// Configuration for reflection engine
#[derive(Debug, Clone)]
pub struct ReflectionConfig {
    /// Minimum quality score to accept (0.0 - 1.0)
    pub min_quality: f64,
    /// Maximum revision attempts
    pub max_revisions: u32,
    /// Whether to evaluate simple responses (commands, short answers)
    pub evaluate_simple: bool,
    /// Minimum response length to evaluate
    pub min_length_to_evaluate: usize,
    /// Dimensions to evaluate
    pub dimensions: Vec<QualityDimension>,
    /// Enable critique generation
    pub generate_critique: bool,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            min_quality: 0.7,
            max_revisions: 2,
            evaluate_simple: false,
            min_length_to_evaluate: 100,
            dimensions: vec![
                QualityDimension::Accuracy,
                QualityDimension::Helpfulness,
                QualityDimension::Safety,
                QualityDimension::Clarity,
                QualityDimension::Instruction,
            ],
            generate_critique: true,
        }
    }
}

/// Reflection engine for self-correction
#[derive(Clone)]
pub struct ReflectionEngine {
    config: ReflectionConfig,
}

impl ReflectionEngine {
    /// Create a new reflection engine
    pub fn new() -> Self {
        Self::with_config(ReflectionConfig::default())
    }

    /// Create with custom config
    pub fn with_config(config: ReflectionConfig) -> Self {
        Self { config }
    }

    /// Check if response should be evaluated
    pub fn should_evaluate(&self, response: &str, is_command: bool) -> bool {
        if is_command && !self.config.evaluate_simple {
            return false;
        }
        response.len() >= self.config.min_length_to_evaluate
    }

    /// Evaluate response quality using LLM-as-judge
    pub async fn evaluate(
        &self,
        prompt: &str,
        response: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<QualityScore> {
        if !llama.is_available().await {
            // Can't evaluate without LLM, assume good
            return Ok(QualityScore::perfect());
        }

        let eval_prompt = format!(
            r#"Evaluate this AI response on a scale of 0-10 for each dimension.
Return ONLY a JSON object with scores and brief critiques.

User prompt: {}

AI response: {}

Evaluate these dimensions:
- accuracy: Is the response factually correct?
- helpfulness: Does it help the user achieve their goal?
- safety: Is it appropriate and safe?
- clarity: Is it well-written and easy to understand?
- instruction_following: Does it follow the user's instructions?

Return JSON:
{{"accuracy": {{"score": 8, "critique": "..."}}, "helpfulness": {{"score": 9, "critique": "..."}}, ...}}

JSON only:"#,
            truncate(prompt, 500),
            truncate(response, 1000)
        );

        let result = llama.generate(&eval_prompt).await?;
        self.parse_evaluation(&result)
    }

    /// Parse LLM evaluation response
    fn parse_evaluation(&self, response: &str) -> Result<QualityScore> {
        // Find JSON in response
        let json_str = extract_json(response).unwrap_or(response);

        #[derive(Deserialize)]
        struct DimEval {
            score: f64,
            critique: Option<String>,
        }

        #[derive(Deserialize)]
        struct EvalResponse {
            accuracy: Option<DimEval>,
            helpfulness: Option<DimEval>,
            safety: Option<DimEval>,
            clarity: Option<DimEval>,
            instruction_following: Option<DimEval>,
        }

        let parsed: EvalResponse = serde_json::from_str(json_str)
            .unwrap_or_else(|_| EvalResponse {
                accuracy: None,
                helpfulness: None,
                safety: None,
                clarity: None,
                instruction_following: None,
            });

        let mut dimensions = Vec::new();

        if let Some(d) = parsed.accuracy {
            dimensions.push(DimensionScore {
                dimension: QualityDimension::Accuracy,
                score: (d.score / 10.0).clamp(0.0, 1.0),
                critique: d.critique,
            });
        }

        if let Some(d) = parsed.helpfulness {
            dimensions.push(DimensionScore {
                dimension: QualityDimension::Helpfulness,
                score: (d.score / 10.0).clamp(0.0, 1.0),
                critique: d.critique,
            });
        }

        if let Some(d) = parsed.safety {
            dimensions.push(DimensionScore {
                dimension: QualityDimension::Safety,
                score: (d.score / 10.0).clamp(0.0, 1.0),
                critique: d.critique,
            });
        }

        if let Some(d) = parsed.clarity {
            dimensions.push(DimensionScore {
                dimension: QualityDimension::Clarity,
                score: (d.score / 10.0).clamp(0.0, 1.0),
                critique: d.critique,
            });
        }

        if let Some(d) = parsed.instruction_following {
            dimensions.push(DimensionScore {
                dimension: QualityDimension::Instruction,
                score: (d.score / 10.0).clamp(0.0, 1.0),
                critique: d.critique,
            });
        }

        if dimensions.is_empty() {
            // Parsing failed, assume acceptable
            Ok(QualityScore::perfect())
        } else {
            Ok(QualityScore::from_dimensions(
                dimensions,
                "Evaluated by LLM-as-judge".to_string(),
            ))
        }
    }

    /// Generate critique and suggestions for improvement
    pub async fn critique(
        &self,
        prompt: &str,
        response: &str,
        quality: &QualityScore,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<String> {
        if !llama.is_available().await {
            return Ok(quality.critique.clone());
        }

        let low_dims: Vec<_> = quality
            .dimensions
            .iter()
            .filter(|d| d.score < 0.7)
            .map(|d| d.dimension.as_str())
            .collect();

        if low_dims.is_empty() {
            return Ok("Response is good quality, no improvements needed.".to_string());
        }

        let critique_prompt = format!(
            r#"The following AI response scored low on: {}

User prompt: {}
AI response: {}

Provide specific, actionable improvements in 2-3 bullet points.
Focus on the weak dimensions. Be constructive.

Improvements:"#,
            low_dims.join(", "),
            truncate(prompt, 300),
            truncate(response, 500)
        );

        llama.generate(&critique_prompt).await
    }

    /// Revise a response based on critique
    pub async fn revise(
        &self,
        prompt: &str,
        original: &str,
        critique: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<String> {
        let revision_prompt = format!(
            r#"Improve this AI response based on the critique.

Original user prompt: {}

Original response: {}

Critique and improvements needed:
{}

Write an improved response that addresses the critique.
Keep the same overall structure but fix the issues.

Improved response:"#,
            prompt,
            original,
            critique
        );

        llama.generate(&revision_prompt).await
    }

    /// Full reflection loop: evaluate → critique → revise (if needed)
    pub async fn reflect(
        &self,
        prompt: &str,
        response: &str,
        llama: &crate::llama_worker::LlamaWorker,
    ) -> Result<ReflectionResult> {
        let mut current_response = response.to_string();
        let mut revision_count = 0;

        // Initial evaluation
        let mut quality = self.evaluate(prompt, &current_response, llama).await?;

        // Revision loop
        while quality.should_retry && revision_count < self.config.max_revisions {
            info!(
                "Response quality {:.0}% below threshold, revising (attempt {})",
                quality.overall * 100.0,
                revision_count + 1
            );

            // Generate critique
            let critique = self.critique(prompt, &current_response, &quality, llama).await?;

            // Revise response
            let revised = self.revise(prompt, &current_response, &critique, llama).await?;

            // Re-evaluate
            let new_quality = self.evaluate(prompt, &revised, llama).await?;

            // Only use revision if it's better
            if new_quality.overall > quality.overall {
                current_response = revised;
                quality = new_quality;
            } else {
                debug!("Revision didn't improve quality, keeping original");
                break;
            }

            revision_count += 1;
        }

        let revised = if revision_count > 0 && current_response != response {
            Some(current_response.clone())
        } else {
            None
        };

        Ok(ReflectionResult {
            original: response.to_string(),
            quality,
            revised,
            revision_count,
            final_response: current_response,
        })
    }

    /// Quick check if response seems problematic (heuristic, no LLM)
    pub fn quick_check(&self, response: &str) -> bool {
        // Check for common issues without LLM
        let issues = [
            response.len() < 10,                                    // Too short
            response.contains("I cannot"),                          // Refusal
            response.contains("I don't have"),                      // Limitation
            response.to_lowercase().contains("error"),              // Error mention
            response.matches("...").count() > 3,                    // Truncation
            response.starts_with("I'm sorry"),                      // Apologetic
        ];

        issues.iter().any(|&x| x)
    }
}

impl Default for ReflectionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract JSON from a string that may contain other text
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

/// Truncate string to max length
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_score_calculation() {
        let dimensions = vec![
            DimensionScore {
                dimension: QualityDimension::Accuracy,
                score: 0.9,
                critique: None,
            },
            DimensionScore {
                dimension: QualityDimension::Helpfulness,
                score: 0.8,
                critique: None,
            },
            DimensionScore {
                dimension: QualityDimension::Safety,
                score: 1.0,
                critique: None,
            },
        ];

        let score = QualityScore::from_dimensions(dimensions, "Test".to_string());
        assert!(score.overall > 0.8);
        assert!(!score.should_retry);
    }

    #[test]
    fn test_should_retry() {
        let dimensions = vec![
            DimensionScore {
                dimension: QualityDimension::Accuracy,
                score: 0.3, // Very low
                critique: Some("Factually incorrect".to_string()),
            },
            DimensionScore {
                dimension: QualityDimension::Helpfulness,
                score: 0.5,
                critique: None,
            },
        ];

        let score = QualityScore::from_dimensions(dimensions, "Test".to_string());
        assert!(score.should_retry);
        assert!(!score.improvements.is_empty());
    }

    #[test]
    fn test_extract_json() {
        let text = "Here is the evaluation: {\"score\": 8} and more text";
        assert_eq!(extract_json(text), Some("{\"score\": 8}"));
    }

    #[test]
    fn test_quick_check() {
        let engine = ReflectionEngine::new();

        assert!(engine.quick_check("I cannot help with that"));
        assert!(engine.quick_check(""));
        assert!(!engine.quick_check("Here is a helpful response with good content."));
    }
}
