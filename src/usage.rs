//! Token Usage Tracking
//!
//! Tracks token usage per user with SQLite storage.
//! Supports daily/monthly limits and cost estimation.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// Token usage record
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub user_id: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub model: String,
    pub timestamp: i64,
}

/// Usage summary for a user
#[derive(Debug, Clone, Default)]
pub struct UsageSummary {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub request_count: i64,
    pub estimated_cost_usd: f64,
}

/// User limits
#[derive(Debug, Clone)]
pub struct UserLimits {
    pub daily_token_limit: Option<i64>,
    pub monthly_token_limit: Option<i64>,
    pub daily_cost_limit_usd: Option<f64>,
    pub monthly_cost_limit_usd: Option<f64>,
}

impl Default for UserLimits {
    fn default() -> Self {
        Self {
            daily_token_limit: None,                 // No daily token limit
            monthly_token_limit: None,               // No monthly token limit
            daily_cost_limit_usd: Some(20.0),        // $20/day (safety)
            monthly_cost_limit_usd: Some(200.0),     // $200/month (subscription limit)
        }
    }
}

/// Usage tracker with SQLite backend
pub struct UsageTracker {
    conn: Mutex<Connection>,
}

impl UsageTracker {
    /// Create or open usage database
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // Create tables
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER DEFAULT 0,
                cache_write_tokens INTEGER DEFAULT 0,
                model TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS user_limits (
                user_id INTEGER PRIMARY KEY,
                daily_token_limit INTEGER,
                monthly_token_limit INTEGER,
                daily_cost_limit_usd REAL,
                monthly_cost_limit_usd REAL
            );

            CREATE INDEX IF NOT EXISTS idx_usage_user_time ON usage(user_id, timestamp);
            "#,
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record token usage
    pub fn record_usage(&self, record: &UsageRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO usage (user_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, model, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.user_id,
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
                record.cache_write_tokens,
                record.model,
                record.timestamp,
            ],
        )?;
        Ok(())
    }

    /// Get usage summary for today
    pub fn get_daily_usage(&self, user_id: i64) -> Result<UsageSummary> {
        let start_of_day = Self::start_of_day();
        self.get_usage_since(user_id, start_of_day)
    }

    /// Get usage summary for this month
    pub fn get_monthly_usage(&self, user_id: i64) -> Result<UsageSummary> {
        let start_of_month = Self::start_of_month();
        self.get_usage_since(user_id, start_of_month)
    }

    /// Get usage summary for all time
    pub fn get_total_usage(&self, user_id: i64) -> Result<UsageSummary> {
        self.get_usage_since(user_id, 0)
    }

    fn get_usage_since(&self, user_id: i64, since_timestamp: i64) -> Result<UsageSummary> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(cache_write_tokens), 0),
                COUNT(*)
             FROM usage
             WHERE user_id = ?1 AND timestamp >= ?2",
        )?;

        let summary = stmt.query_row(params![user_id, since_timestamp], |row| {
            Ok(UsageSummary {
                total_input_tokens: row.get(0)?,
                total_output_tokens: row.get(1)?,
                total_cache_read_tokens: row.get(2)?,
                total_cache_write_tokens: row.get(3)?,
                request_count: row.get(4)?,
                estimated_cost_usd: 0.0,
            })
        })?;

        // Calculate cost
        let mut summary = summary;
        summary.estimated_cost_usd = Self::estimate_cost(&summary);
        Ok(summary)
    }

    /// Get user limits
    pub fn get_user_limits(&self, user_id: i64) -> Result<UserLimits> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT daily_token_limit, monthly_token_limit, daily_cost_limit_usd, monthly_cost_limit_usd
             FROM user_limits WHERE user_id = ?1",
        )?;

        match stmt.query_row(params![user_id], |row| {
            Ok(UserLimits {
                daily_token_limit: row.get(0)?,
                monthly_token_limit: row.get(1)?,
                daily_cost_limit_usd: row.get(2)?,
                monthly_cost_limit_usd: row.get(3)?,
            })
        }) {
            Ok(limits) => Ok(limits),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(UserLimits::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Set user limits
    pub fn set_user_limits(&self, user_id: i64, limits: &UserLimits) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO user_limits
             (user_id, daily_token_limit, monthly_token_limit, daily_cost_limit_usd, monthly_cost_limit_usd)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                user_id,
                limits.daily_token_limit,
                limits.monthly_token_limit,
                limits.daily_cost_limit_usd,
                limits.monthly_cost_limit_usd,
            ],
        )?;
        Ok(())
    }

    /// Check if user is within limits
    pub fn check_limits(&self, user_id: i64) -> Result<LimitCheck> {
        let limits = self.get_user_limits(user_id)?;
        let daily = self.get_daily_usage(user_id)?;
        let monthly = self.get_monthly_usage(user_id)?;

        let daily_tokens = daily.total_input_tokens + daily.total_output_tokens;
        let monthly_tokens = monthly.total_input_tokens + monthly.total_output_tokens;

        // Check token limits
        if let Some(limit) = limits.daily_token_limit {
            if daily_tokens >= limit {
                return Ok(LimitCheck::Exceeded(LimitType::DailyTokens {
                    used: daily_tokens,
                    limit,
                }));
            }
        }

        if let Some(limit) = limits.monthly_token_limit {
            if monthly_tokens >= limit {
                return Ok(LimitCheck::Exceeded(LimitType::MonthlyTokens {
                    used: monthly_tokens,
                    limit,
                }));
            }
        }

        // Check cost limits
        if let Some(limit) = limits.daily_cost_limit_usd {
            if daily.estimated_cost_usd >= limit {
                return Ok(LimitCheck::Exceeded(LimitType::DailyCost {
                    used: daily.estimated_cost_usd,
                    limit,
                }));
            }
        }

        if let Some(limit) = limits.monthly_cost_limit_usd {
            if monthly.estimated_cost_usd >= limit {
                return Ok(LimitCheck::Exceeded(LimitType::MonthlyCost {
                    used: monthly.estimated_cost_usd,
                    limit,
                }));
            }
        }

        // Calculate remaining
        let remaining = LimitRemaining {
            daily_tokens: limits.daily_token_limit.map(|l| l - daily_tokens),
            monthly_tokens: limits.monthly_token_limit.map(|l| l - monthly_tokens),
            daily_cost_usd: limits.daily_cost_limit_usd.map(|l| l - daily.estimated_cost_usd),
            monthly_cost_usd: limits.monthly_cost_limit_usd.map(|l| l - monthly.estimated_cost_usd),
        };

        Ok(LimitCheck::Ok(remaining))
    }

    /// Estimate cost based on Sonnet pricing (default)
    fn estimate_cost(summary: &UsageSummary) -> f64 {
        // Claude Sonnet 4 pricing (per million tokens)
        const INPUT_PRICE: f64 = 3.0;
        const OUTPUT_PRICE: f64 = 15.0;
        const CACHE_READ_DISCOUNT: f64 = 0.1; // 90% cheaper

        let input_cost = (summary.total_input_tokens as f64 / 1_000_000.0) * INPUT_PRICE;
        let output_cost = (summary.total_output_tokens as f64 / 1_000_000.0) * OUTPUT_PRICE;
        let cache_savings = (summary.total_cache_read_tokens as f64 / 1_000_000.0)
            * INPUT_PRICE * (1.0 - CACHE_READ_DISCOUNT);

        input_cost + output_cost - cache_savings
    }

    fn start_of_day() -> i64 {
        use chrono::{Local, Timelike};
        let now = Local::now();
        let start = now
            .with_hour(0).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap();
        start.timestamp()
    }

    fn start_of_month() -> i64 {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let start = now
            .with_day(1).unwrap()
            .with_hour(0).unwrap()
            .with_minute(0).unwrap()
            .with_second(0).unwrap()
            .with_nanosecond(0).unwrap();
        start.timestamp()
    }
}

/// Result of limit check
#[derive(Debug)]
pub enum LimitCheck {
    Ok(LimitRemaining),
    Exceeded(LimitType),
}

/// Type of limit exceeded
#[derive(Debug)]
pub enum LimitType {
    DailyTokens { used: i64, limit: i64 },
    MonthlyTokens { used: i64, limit: i64 },
    DailyCost { used: f64, limit: f64 },
    MonthlyCost { used: f64, limit: f64 },
}

/// Remaining limits
#[derive(Debug)]
pub struct LimitRemaining {
    pub daily_tokens: Option<i64>,
    pub monthly_tokens: Option<i64>,
    pub daily_cost_usd: Option<f64>,
    pub monthly_cost_usd: Option<f64>,
}

impl LimitType {
    pub fn message(&self) -> String {
        match self {
            LimitType::DailyTokens { used, limit } => {
                format!("Daily token limit reached: {}/{}", format_tokens(*used), format_tokens(*limit))
            }
            LimitType::MonthlyTokens { used, limit } => {
                format!("Monthly token limit reached: {}/{}", format_tokens(*used), format_tokens(*limit))
            }
            LimitType::DailyCost { used, limit } => {
                format!("Daily cost limit reached: ${:.2}/${:.2}", used, limit)
            }
            LimitType::MonthlyCost { used, limit } => {
                format!("Monthly cost limit reached: ${:.2}/${:.2}", used, limit)
            }
        }
    }
}

/// Format token count for display (e.g., 1.5M, 500K)
pub fn format_tokens(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_record_and_query_usage() {
        let temp = NamedTempFile::new().unwrap();
        let tracker = UsageTracker::new(temp.path()).unwrap();

        let record = UsageRecord {
            user_id: 12345,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 100,
            model: "claude-sonnet-4".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        tracker.record_usage(&record).unwrap();

        let summary = tracker.get_total_usage(12345).unwrap();
        assert_eq!(summary.total_input_tokens, 1000);
        assert_eq!(summary.total_output_tokens, 500);
        assert_eq!(summary.request_count, 1);
    }

    #[test]
    fn test_limit_check() {
        let temp = NamedTempFile::new().unwrap();
        let tracker = UsageTracker::new(temp.path()).unwrap();

        // Set low limit
        tracker.set_user_limits(12345, &UserLimits {
            daily_token_limit: Some(100),
            monthly_token_limit: Some(1000),
            daily_cost_limit_usd: None,
            monthly_cost_limit_usd: None,
        }).unwrap();

        // Record usage that exceeds daily limit
        let record = UsageRecord {
            user_id: 12345,
            input_tokens: 80,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: "claude-sonnet-4".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        tracker.record_usage(&record).unwrap();

        match tracker.check_limits(12345).unwrap() {
            LimitCheck::Exceeded(LimitType::DailyTokens { .. }) => {}
            _ => panic!("Expected daily limit exceeded"),
        }
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn test_cost_estimation() {
        let summary = UsageSummary {
            total_input_tokens: 1_000_000,  // 1M input = $3
            total_output_tokens: 100_000,   // 100K output = $1.5
            total_cache_read_tokens: 0,
            total_cache_write_tokens: 0,
            request_count: 10,
            estimated_cost_usd: 0.0,
        };

        let cost = UsageTracker::estimate_cost(&summary);
        assert!((cost - 4.5).abs() < 0.01); // $3 + $1.5 = $4.5
    }
}
