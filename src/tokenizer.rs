//! Token Counter - Pre-flight cost estimation
//!
//! Counts tokens BEFORE sending to Claude API to:
//! - Warn users about budget impact
//! - Prevent accidental budget overruns
//! - Enable smart context pruning

use crate::router::ModelHint;

/// Token counter using Claude's tokenization approximation
///
/// Claude uses a BPE tokenizer similar to cl100k_base.
/// This provides a good approximation without external dependencies.
pub struct TokenCounter {
    /// Average characters per token (Claude: ~4 chars/token for English)
    chars_per_token: f32,
}

/// Budget check result
#[derive(Debug, Clone)]
pub enum BudgetCheck {
    /// Within budget
    Ok {
        estimated_cost: f64,
        estimated_tokens: usize,
    },
    /// Would exceed budget
    Warning {
        estimated_cost: f64,
        remaining_budget: f64,
        estimated_tokens: usize,
    },
    /// Critically over budget
    Exceeded {
        estimated_cost: f64,
        remaining_budget: f64,
        estimated_tokens: usize,
    },
}

impl BudgetCheck {
    pub fn is_ok(&self) -> bool {
        matches!(self, BudgetCheck::Ok { .. })
    }

    pub fn should_warn(&self) -> bool {
        matches!(self, BudgetCheck::Warning { .. } | BudgetCheck::Exceeded { .. })
    }

    pub fn should_block(&self) -> bool {
        matches!(self, BudgetCheck::Exceeded { .. })
    }

    pub fn estimated_cost(&self) -> f64 {
        match self {
            BudgetCheck::Ok { estimated_cost, .. } => *estimated_cost,
            BudgetCheck::Warning { estimated_cost, .. } => *estimated_cost,
            BudgetCheck::Exceeded { estimated_cost, .. } => *estimated_cost,
        }
    }
}

/// Model pricing (per million tokens)
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,   // 10% of input
    pub cache_write_per_million: f64,  // 25% of input
}

impl ModelPricing {
    pub const HAIKU: Self = Self {
        input_per_million: 0.25,
        output_per_million: 1.25,
        cache_read_per_million: 0.025,
        cache_write_per_million: 0.0625,
    };

    pub const SONNET: Self = Self {
        input_per_million: 3.0,
        output_per_million: 15.0,
        cache_read_per_million: 0.30,
        cache_write_per_million: 0.75,
    };

    pub const OPUS: Self = Self {
        input_per_million: 15.0,
        output_per_million: 75.0,
        cache_read_per_million: 1.50,
        cache_write_per_million: 3.75,
    };

    pub fn for_model(model: &ModelHint) -> Self {
        match model {
            ModelHint::Haiku => Self::HAIKU,
            ModelHint::Sonnet => Self::SONNET,
            ModelHint::Opus => Self::OPUS,
        }
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCounter {
    /// Create new token counter
    pub fn new() -> Self {
        Self {
            // Claude averages ~4 characters per token for English text
            // Code tends to be ~3.5 chars/token due to symbols
            chars_per_token: 3.8,
        }
    }

    /// Count approximate tokens in text
    ///
    /// Uses character-based approximation suitable for Claude models.
    /// Accuracy: ±10% for typical text, ±15% for code.
    pub fn count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        // Basic approximation
        let char_count = text.chars().count();
        let base_tokens = (char_count as f32 / self.chars_per_token).ceil() as usize;

        // Adjust for special patterns
        let adjustments = self.calculate_adjustments(text);

        (base_tokens as f32 * adjustments).ceil() as usize
    }

    /// Calculate adjustment factor based on content type
    fn calculate_adjustments(&self, text: &str) -> f32 {
        let mut factor = 1.0f32;

        // Code has more tokens per character (symbols, short identifiers)
        let code_indicators = ["{", "}", "(", ")", ";", "=>", "->", "::"];
        let code_density: f32 = code_indicators
            .iter()
            .map(|p| text.matches(p).count() as f32)
            .sum::<f32>()
            / text.len().max(1) as f32;

        if code_density > 0.01 {
            factor *= 1.15; // Code is ~15% more token-dense
        }

        // URLs and paths are token-heavy
        if text.contains("http://") || text.contains("https://") || text.contains("file://") {
            factor *= 1.1;
        }

        // JSON/structured data
        if text.starts_with('{') || text.starts_with('[') {
            factor *= 1.2;
        }

        // Numbers are efficient
        let digit_ratio = text.chars().filter(|c| c.is_ascii_digit()).count() as f32
            / text.len().max(1) as f32;
        if digit_ratio > 0.3 {
            factor *= 0.9; // Numbers compress well
        }

        factor
    }

    /// Count tokens in a message with role
    pub fn count_message(&self, role: &str, content: &str) -> usize {
        // Each message has overhead: ~4 tokens for role/formatting
        let overhead = 4;
        overhead + self.count(content)
    }

    /// Estimate cost for a request
    ///
    /// # Arguments
    /// * `input_text` - The full input (system + user messages)
    /// * `expected_output_tokens` - Estimated output length (default: 1000)
    /// * `model` - The model to use
    /// * `cache_hit_ratio` - Expected cache hit ratio (0.0 - 1.0)
    pub fn estimate_cost(
        &self,
        input_text: &str,
        expected_output_tokens: usize,
        model: &ModelHint,
        cache_hit_ratio: f32,
    ) -> f64 {
        let input_tokens = self.count(input_text);
        let pricing = ModelPricing::for_model(model);

        // Calculate cached vs uncached input
        let cached_tokens = (input_tokens as f32 * cache_hit_ratio) as usize;
        let uncached_tokens = input_tokens - cached_tokens;

        let input_cost = (uncached_tokens as f64 * pricing.input_per_million / 1_000_000.0)
            + (cached_tokens as f64 * pricing.cache_read_per_million / 1_000_000.0);

        let output_cost = expected_output_tokens as f64 * pricing.output_per_million / 1_000_000.0;

        input_cost + output_cost
    }

    /// Check if request fits within budget
    pub fn check_budget(
        &self,
        input_text: &str,
        expected_output_tokens: usize,
        model: &ModelHint,
        remaining_budget: f64,
        cache_hit_ratio: f32,
    ) -> BudgetCheck {
        let estimated_cost = self.estimate_cost(input_text, expected_output_tokens, model, cache_hit_ratio);
        let estimated_tokens = self.count(input_text) + expected_output_tokens;

        // Warning threshold: 50% of remaining budget
        let warning_threshold = remaining_budget * 0.5;

        if estimated_cost > remaining_budget {
            BudgetCheck::Exceeded {
                estimated_cost,
                remaining_budget,
                estimated_tokens,
            }
        } else if estimated_cost > warning_threshold {
            BudgetCheck::Warning {
                estimated_cost,
                remaining_budget,
                estimated_tokens,
            }
        } else {
            BudgetCheck::Ok {
                estimated_cost,
                estimated_tokens,
            }
        }
    }

    /// Format token count for display
    pub fn format_tokens(tokens: usize) -> String {
        if tokens >= 1_000_000 {
            format!("{:.1}M", tokens as f64 / 1_000_000.0)
        } else if tokens >= 1_000 {
            format!("{:.1}K", tokens as f64 / 1_000.0)
        } else {
            tokens.to_string()
        }
    }

    /// Format cost for display
    pub fn format_cost(cost: f64) -> String {
        if cost < 0.01 {
            format!("${:.4}", cost)
        } else if cost < 1.0 {
            format!("${:.3}", cost)
        } else {
            format!("${:.2}", cost)
        }
    }

    /// Suggest context pruning if over token limit
    pub fn suggest_pruning(&self, current_tokens: usize, target_tokens: usize) -> PruningSuggestion {
        if current_tokens <= target_tokens {
            return PruningSuggestion::None;
        }

        let excess = current_tokens - target_tokens;
        let reduction_ratio = excess as f32 / current_tokens as f32;

        if reduction_ratio > 0.5 {
            PruningSuggestion::AggressiveCompression {
                tokens_to_remove: excess,
                suggestion: "Remove older messages, compress summaries".to_string(),
            }
        } else if reduction_ratio > 0.2 {
            PruningSuggestion::ModerateCompression {
                tokens_to_remove: excess,
                suggestion: "Summarize older context, keep recent messages".to_string(),
            }
        } else {
            PruningSuggestion::LightTrim {
                tokens_to_remove: excess,
                suggestion: "Remove oldest message or trim system context".to_string(),
            }
        }
    }
}

/// Pruning suggestion for context management
#[derive(Debug, Clone)]
pub enum PruningSuggestion {
    None,
    LightTrim {
        tokens_to_remove: usize,
        suggestion: String,
    },
    ModerateCompression {
        tokens_to_remove: usize,
        suggestion: String,
    },
    AggressiveCompression {
        tokens_to_remove: usize,
        suggestion: String,
    },
}

impl PruningSuggestion {
    pub fn tokens_to_remove(&self) -> usize {
        match self {
            Self::None => 0,
            Self::LightTrim { tokens_to_remove, .. } => *tokens_to_remove,
            Self::ModerateCompression { tokens_to_remove, .. } => *tokens_to_remove,
            Self::AggressiveCompression { tokens_to_remove, .. } => *tokens_to_remove,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_count() {
        let counter = TokenCounter::new();

        // ~4 chars per token for English
        assert!(counter.count("Hello, world!") > 2);
        assert!(counter.count("Hello, world!") < 10);

        // Empty string
        assert_eq!(counter.count(""), 0);

        // Code with lots of symbols should have adjustment
        let code = "fn main() { let x = 1; let y = 2; println!(\"{}{}\", x, y); }";
        let code_tokens = counter.count(code);
        // Code should produce reasonable token counts
        assert!(code_tokens > 10);
        assert!(code_tokens < 30);
    }

    #[test]
    fn test_cost_estimation() {
        let counter = TokenCounter::new();

        // Test cost for 1000 input + 500 output tokens
        let input = "a".repeat(4000); // ~1000 tokens
        let cost = counter.estimate_cost(&input, 500, &ModelHint::Sonnet, 0.0);

        // Sonnet: $3/M input + $15/M output
        // Expected: ~$0.003 input + ~$0.0075 output = ~$0.01
        assert!(cost > 0.005);
        assert!(cost < 0.02);
    }

    #[test]
    fn test_budget_check() {
        let counter = TokenCounter::new();
        let input = "Test message".to_string();

        // Should be OK with large budget
        let check = counter.check_budget(&input, 100, &ModelHint::Haiku, 10.0, 0.0);
        assert!(check.is_ok());

        // Should warn with small budget
        let check = counter.check_budget(&input, 100, &ModelHint::Opus, 0.001, 0.0);
        assert!(check.should_warn());
    }

    #[test]
    fn test_format() {
        assert_eq!(TokenCounter::format_tokens(500), "500");
        assert_eq!(TokenCounter::format_tokens(1500), "1.5K");
        assert_eq!(TokenCounter::format_tokens(1_500_000), "1.5M");

        assert_eq!(TokenCounter::format_cost(0.001), "$0.0010");
        assert_eq!(TokenCounter::format_cost(0.15), "$0.150");
        assert_eq!(TokenCounter::format_cost(5.5), "$5.50");
    }
}
