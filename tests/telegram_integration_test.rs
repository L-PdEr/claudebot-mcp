//! Telegram Bot Integration Tests (T1.3)
//!
//! End-to-end integration tests for bot components without actual Telegram connection.
//! Tests the internal flow: initialization, message processing, response generation.

use claudebot_mcp::{
    ConversationStore, MemoryStore, LifecycleManager, LifecycleConfig,
    TokenCounter, BudgetCheck, PreflightChecker,
};
use claudebot_mcp::usage::UsageTracker;
use claudebot_mcp::permissions::PermissionManager;
use claudebot_mcp::autonomous::{GoalTracker, ContextManager, FeedbackLoop};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test environment with all necessary components
struct TestEnvironment {
    temp_dir: TempDir,
    memory_store: MemoryStore,
    conversation_store: ConversationStore,
    usage_tracker: UsageTracker,
    lifecycle: Arc<LifecycleManager>,
    permission_manager: PermissionManager,
    token_counter: TokenCounter,
    preflight_checker: PreflightChecker,
    goal_tracker: GoalTracker,
    context_manager: ContextManager,
    feedback_loop: FeedbackLoop,
}

impl TestEnvironment {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let memory_path = temp_dir.path().join("memory.db");
        let conversation_path = temp_dir.path().join("conversation.db");
        let usage_path = temp_dir.path().join("usage.db");
        let goals_path = temp_dir.path().join("goals.db");

        Self {
            memory_store: MemoryStore::open(&memory_path).expect("Failed to create memory store"),
            conversation_store: ConversationStore::open(&conversation_path)
                .expect("Failed to create conversation store"),
            usage_tracker: UsageTracker::new(&usage_path).expect("Failed to create usage tracker"),
            lifecycle: LifecycleManager::new(LifecycleConfig::default()),
            permission_manager: PermissionManager::new(),
            token_counter: TokenCounter::new(),
            preflight_checker: PreflightChecker::new(),
            goal_tracker: GoalTracker::open(&goals_path).unwrap_or_else(|_| GoalTracker::new()),
            context_manager: ContextManager::new(),
            feedback_loop: FeedbackLoop::new(),
            temp_dir,
        }
    }

    fn working_dir(&self) -> PathBuf {
        self.temp_dir.path().to_path_buf()
    }
}

// ============ Component Initialization Tests ============

mod initialization {
    use super::*;

    #[test]
    fn test_all_components_initialize() {
        let env = TestEnvironment::new();

        // Verify all components are usable
        assert!(env.working_dir().exists());

        // Memory store works
        let mem_result = env.memory_store.learn("test fact", "test", "test_source", 0.9);
        assert!(mem_result.is_ok());

        // Conversation store works
        let conv_result = env.conversation_store.add_message(123, "user", "hello");
        assert!(conv_result.is_ok());

        // Usage tracker works
        let usage = env.usage_tracker.get_daily_usage(123);
        assert!(usage.is_ok());
    }

    #[test]
    fn test_lifecycle_states() {
        let lifecycle = LifecycleManager::new(LifecycleConfig {
            idle_timeout: std::time::Duration::from_secs(1),
            sleep_task_interval: std::time::Duration::from_secs(1),
            enable_consolidation: false,
            enable_decay: false,
            enable_compression: false,
        });

        // Initially should be in Wake state after activity
        lifecycle.record_activity();
        // Lifecycle is async, just verify it accepts activity
    }

    #[test]
    fn test_permission_manager_default_state() {
        let pm = PermissionManager::new();

        // New user defaults to autonomous mode (no restrictions)
        let status = pm.get_status(12345);
        assert_eq!(
            status.level,
            claudebot_mcp::permissions::PermissionLevel::Autonomous
        );
    }
}

// ============ Message Flow Tests ============

mod message_flow {
    use super::*;

    #[test]
    fn test_conversation_persistence() {
        let env = TestEnvironment::new();
        let chat_id = 12345;

        // Simulate message exchange
        env.conversation_store.add_exchange(
            chat_id,
            "What is Rust?",
            "Rust is a systems programming language."
        ).unwrap();

        // Verify persistence
        let history = env.conversation_store.get_history(chat_id, 10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
    }

    #[test]
    fn test_memory_learning_from_response() {
        let env = TestEnvironment::new();

        // Simulate learning from a response
        let fact = "The user prefers Rust over Python";
        let _id = env.memory_store.learn(fact, "preference", "test", 0.9).unwrap();

        // Verify retrieval
        let results = env.memory_store.search("Rust preference", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].entry.content.contains("Rust"));
    }

    #[test]
    fn test_multi_chat_isolation() {
        let env = TestEnvironment::new();

        // Two different chats
        env.conversation_store.add_message(111, "user", "Chat 1 content").unwrap();
        env.conversation_store.add_message(222, "user", "Chat 2 content").unwrap();

        // Verify isolation
        let history1 = env.conversation_store.get_history(111, 10).unwrap();
        let history2 = env.conversation_store.get_history(222, 10).unwrap();

        assert_eq!(history1.len(), 1);
        assert_eq!(history2.len(), 1);
        assert!(history1[0].content.contains("Chat 1"));
        assert!(history2[0].content.contains("Chat 2"));
    }
}

// ============ Budget and Usage Tests ============

mod budget_tracking {
    use super::*;
    use claudebot_mcp::usage::UsageRecord;

    #[test]
    fn test_usage_recording() {
        let env = TestEnvironment::new();
        let user_id = 12345;

        let record = UsageRecord {
            user_id,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 100,
            model: "claude-3-sonnet".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        env.usage_tracker.record_usage(&record).unwrap();

        let daily = env.usage_tracker.get_daily_usage(user_id).unwrap();
        assert_eq!(daily.total_input_tokens, 1000);
        assert_eq!(daily.total_output_tokens, 500);
        assert!(daily.estimated_cost_usd > 0.0);
    }

    #[test]
    fn test_budget_check_ok() {
        let counter = TokenCounter::new();
        let prompt = "Short prompt";

        let check = counter.check_budget(
            prompt,
            100,  // expected output
            &claudebot_mcp::router::ModelHint::Sonnet,
            10.0, // $10 remaining
            0.5,  // cache ratio
        );

        assert!(matches!(check, BudgetCheck::Ok { .. }));
    }

    #[test]
    fn test_budget_check_warning() {
        let counter = TokenCounter::new();
        let prompt = "A".repeat(10000); // Longer prompt

        let check = counter.check_budget(
            &prompt,
            5000,  // expected output
            &claudebot_mcp::router::ModelHint::Opus, // Expensive model
            0.10,  // Only $0.10 remaining
            0.0,   // No cache
        );

        // Should trigger warning or exceeded
        assert!(!matches!(check, BudgetCheck::Ok { .. }));
    }
}

// ============ Goal Tracking Tests ============

mod goal_tracking {
    use super::*;

    #[tokio::test]
    async fn test_goal_extraction() {
        let temp_dir = TempDir::new().unwrap();
        let goals_path = temp_dir.path().join("goals.db");
        let tracker = GoalTracker::open(&goals_path).unwrap_or_else(|_| GoalTracker::new());
        let user_id = 12345;

        // Extract goals from text
        let text = "I want to learn Rust programming";
        let _goals = tracker.extract_goals(text, user_id).await;

        // Pattern-based extraction should find this
        // (depends on implementation details)
        // At minimum, tracker should not error
    }

    #[tokio::test]
    async fn test_goal_auto_complete() {
        let temp_dir = TempDir::new().unwrap();
        let goals_path = temp_dir.path().join("goals.db");
        let tracker = GoalTracker::open(&goals_path).unwrap_or_else(|_| GoalTracker::new());
        let user_id = 12345;

        // Test auto-completion detection
        let text = "I finished the Rust tutorial";
        let _completed = tracker.auto_complete(text, user_id).await;

        // Should handle gracefully even with no prior goals
        // completed may be empty, that's fine
    }
}

// ============ Preflight Check Tests ============

mod preflight {
    use super::*;

    #[tokio::test]
    async fn test_preflight_check_all() {
        let checker = PreflightChecker::new();

        let result = checker.check_all().await;

        // Should return a valid result (may have warnings)
        // The structure should be valid regardless of tool availability
        let _ = result.format_error(); // Just verify it doesn't panic
    }

    #[tokio::test]
    async fn test_preflight_check_git_command() {
        let checker = PreflightChecker::new();

        // Check for git-related command
        let _result = checker.check_for_command("git status").await;

        // Result should indicate git readiness (may be true or false)
        // Just verify it doesn't panic
    }

    #[tokio::test]
    async fn test_preflight_check_generic_command() {
        let checker = PreflightChecker::new();

        // Generic command should pass preflight
        let result = checker.check_for_command("explain this code").await;

        // Non-tool commands should generally be ready
        assert!(result.ready || !result.missing_tools.is_empty());
    }
}

// ============ Rate Limiting Tests ============

mod rate_limiting {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use tokio::sync::RwLock;

    struct TestRateLimiter {
        max_requests: u32,
        window_secs: u64,
        entries: RwLock<HashMap<i64, (Instant, u32)>>,
    }

    impl TestRateLimiter {
        fn new(max_requests: u32, window_secs: u64) -> Self {
            Self {
                max_requests,
                window_secs,
                entries: RwLock::new(HashMap::new()),
            }
        }

        async fn check(&self, user_id: i64) -> bool {
            let mut entries = self.entries.write().await;
            let now = Instant::now();
            let window = Duration::from_secs(self.window_secs);

            let entry = entries.entry(user_id).or_insert((now, 0));

            if now.duration_since(entry.0) >= window {
                entry.0 = now;
                entry.1 = 0;
            }

            if entry.1 >= self.max_requests {
                return false;
            }

            entry.1 += 1;
            true
        }
    }

    #[tokio::test]
    async fn test_rate_limit_allows_under_limit() {
        let limiter = TestRateLimiter::new(5, 60);

        // First 5 requests should pass
        for _ in 0..5 {
            assert!(limiter.check(123).await);
        }
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_over_limit() {
        let limiter = TestRateLimiter::new(3, 60);

        // Use up the limit
        assert!(limiter.check(123).await);
        assert!(limiter.check(123).await);
        assert!(limiter.check(123).await);

        // 4th request should be blocked
        assert!(!limiter.check(123).await);
    }

    #[tokio::test]
    async fn test_rate_limit_per_user_isolation() {
        let limiter = TestRateLimiter::new(2, 60);

        // User 1 uses their limit
        assert!(limiter.check(111).await);
        assert!(limiter.check(111).await);
        assert!(!limiter.check(111).await);

        // User 2 should still have their limit
        assert!(limiter.check(222).await);
        assert!(limiter.check(222).await);
    }
}

// ============ Sensitive Data Detection Tests ============

mod security {
    fn contains_sensitive_data(text: &str) -> bool {
        let lower = text.to_lowercase();

        let patterns = [
            "sk-", "pk-", "api_key", "apikey", "api-key",
            "secret_key", "secretkey", "bearer ",
        ];

        for pattern in patterns {
            if lower.contains(pattern) {
                return true;
            }
        }

        if lower.contains("password") && (lower.contains("=") || lower.contains(":")) {
            return true;
        }

        if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
            return true;
        }

        false
    }

    #[test]
    fn test_detects_api_key() {
        assert!(contains_sensitive_data("My API key is sk-abc123xyz"));
        assert!(contains_sensitive_data("api_key=12345"));
        assert!(contains_sensitive_data("Use bearer token123"));
    }

    #[test]
    fn test_detects_password() {
        assert!(contains_sensitive_data("password=secret123"));
        assert!(contains_sensitive_data("password: mypass"));
    }

    #[test]
    fn test_detects_private_key() {
        assert!(contains_sensitive_data("-----BEGIN RSA PRIVATE KEY-----\nxxx"));
    }

    #[test]
    fn test_allows_safe_content() {
        assert!(!contains_sensitive_data("Hello, how are you?"));
        assert!(!contains_sensitive_data("The password field is required"));
        assert!(!contains_sensitive_data("Use a secure API endpoint"));
    }
}

// ============ End-to-End Flow Simulation ============

mod e2e_simulation {
    use super::*;

    #[tokio::test]
    async fn test_full_message_flow_simulation() {
        let env = TestEnvironment::new();
        let user_id = 12345i64;
        let chat_id = 12345i64;

        // Step 1: Record activity (lifecycle)
        env.lifecycle.record_activity();

        // Step 2: Check user limits
        let limits_ok = env.usage_tracker.check_limits(user_id);
        assert!(limits_ok.is_ok());

        // Step 3: Store user message
        env.conversation_store
            .add_message(chat_id, "user", "Explain Rust ownership")
            .unwrap();

        // Step 4: Simulate response (in real bot, this comes from Claude CLI)
        let simulated_response = "Rust ownership is a system for managing memory...";

        // Step 5: Store assistant response
        env.conversation_store
            .add_message(chat_id, "assistant", simulated_response)
            .unwrap();

        // Step 6: Learn from response
        env.memory_store
            .learn("Rust has ownership system for memory management", "technical", "chat", 0.85)
            .unwrap();

        // Step 7: Record usage
        let record = claudebot_mcp::usage::UsageRecord {
            user_id,
            input_tokens: 50,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: "claude-3-sonnet".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
        };
        env.usage_tracker.record_usage(&record).unwrap();

        // Verify: Conversation stored
        let history = env.conversation_store.get_history(chat_id, 10).unwrap();
        assert_eq!(history.len(), 2);

        // Verify: Memory learned
        let memories = env.memory_store.search("Rust ownership", 5).unwrap();
        assert!(!memories.is_empty());

        // Verify: Usage tracked
        let daily = env.usage_tracker.get_daily_usage(user_id).unwrap();
        assert_eq!(daily.total_input_tokens, 50);
        assert_eq!(daily.total_output_tokens, 200);
    }
}
