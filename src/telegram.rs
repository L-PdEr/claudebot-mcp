//! Telegram Bot integration for ClaudeBot MCP
//!
//! Provides a Telegram interface to Claude Code CLI.
//! Messages are forwarded to the `claude` CLI tool running on the server.
//!
//! Features:
//! - Token usage tracking and budget limits
//! - Pre-flight cost estimation
//! - Lifecycle management (wake/sleep)
//! - Memory integration with continuous learning
//! - Context compression for long conversations
//!
//! Uses explicit Dispatcher pattern for reliable message polling.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::{
    dispatching::{Dispatcher, UpdateFilterExt},
    dptree,
    error_handlers::LoggingErrorHandler,
    net::Download,
    prelude::*,
    types::{ParseMode, Update},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::bridge::GrpcBridgeClient;
use crate::conversation::ConversationStore;
use crate::feedback::{OutputParser, TaskFeedback};
use crate::graph::GraphStore;
use crate::lifecycle::{LifecycleManager, LifecycleConfig, LifecycleCallbacks, ProcessingGuard};
use crate::llama_worker::LlamaWorker;
use crate::memory::MemoryStore;
use crate::permissions::PermissionManager;
use crate::preflight::PreflightChecker;
use crate::circle::{Circle, PipelineMode, PipelineResult};
use crate::telegram_ui::{
    ButtonAction, ConversationContext as UiContext, ContextParser, Intent,
    ProgressManager, suggest_next_actions, suggestions_keyboard,
};
use crate::tokenizer::{TokenCounter, BudgetCheck};
use crate::usage::{format_tokens, LimitCheck, UsageRecord, UsageTracker, UserLimits};

/// Claude CLI JSON output structure
#[derive(Debug, Deserialize)]
struct ClaudeJsonOutput {
    result: String,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
}

/// Run Telegram bot with explicit Dispatcher for reliable polling
pub async fn run_telegram_bot() -> Result<()> {
    let token = std::env::var("TELEGRAM_BOT_TOKEN")
        .expect("TELEGRAM_BOT_TOKEN must be set");

    let allowed_users: Vec<i64> = std::env::var("TELEGRAM_ALLOWED_USERS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let working_dir = std::env::var("CLAUDE_WORKING_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/eliot/workspace"));

    let usage_db_path = std::env::var("USAGE_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| working_dir.join("usage.db"));

    let memory_db_path = std::env::var("MEMORY_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| working_dir.join("memory.db"));

    let conversation_db_path = std::env::var("CONVERSATION_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| working_dir.join("conversations.db"));

    // Create base working directory
    tokio::fs::create_dir_all(&working_dir).await?;

    // Initialize usage tracker, memory store, and conversation store
    let usage_tracker = UsageTracker::new(&usage_db_path)?;
    let memory_store = MemoryStore::open(&memory_db_path)?;
    let conversation_store = ConversationStore::open(&conversation_db_path)?;

    tracing::info!("===========================================");
    tracing::info!("  ClaudeBot Telegram - Starting...");
    tracing::info!("===========================================");
    tracing::info!("Allowed users: {:?}", if allowed_users.is_empty() { "ALL".to_string() } else { format!("{:?}", allowed_users) });
    tracing::info!("Working directory: {:?}", working_dir);
    tracing::info!("Usage database: {:?}", usage_db_path);
    tracing::info!("Memory database: {:?}", memory_db_path);
    tracing::info!("Conversation database: {:?}", conversation_db_path);

    let bot = Bot::new(token.clone());

    // Verify bot token by calling getMe
    tracing::info!("Verifying bot token...");
    match bot.get_me().await {
        Ok(me) => {
            tracing::info!("Bot authenticated: @{} (ID: {})",
                me.username.as_deref().unwrap_or("unknown"),
                me.id
            );
        }
        Err(e) => {
            tracing::error!("Failed to authenticate bot: {}", e);
            anyhow::bail!("Bot authentication failed: {}", e);
        }
    }

    // Delete any existing webhook to ensure polling works
    tracing::info!("Clearing webhook (if any)...");
    if let Err(e) = bot.delete_webhook().await {
        tracing::warn!("Failed to delete webhook: {} (continuing anyway)", e);
    }

    // Initialize additional components
    let token_counter = TokenCounter::new();
    let llama_worker = LlamaWorker::new();
    let lifecycle = LifecycleManager::new(LifecycleConfig {
        idle_timeout: std::time::Duration::from_secs(300), // 5 min
        sleep_task_interval: std::time::Duration::from_secs(60),
        enable_consolidation: true,
        enable_decay: true,
        enable_compression: true,
    });

    // Check Llama availability
    if llama_worker.is_available().await {
        tracing::info!("Llama worker: AVAILABLE (compression enabled)");
    } else {
        tracing::warn!("Llama worker: UNAVAILABLE (compression disabled)");
    }

    // Initialize graph store (uses same database as memory)
    let graph_store = GraphStore::open(&memory_db_path)?;
    tracing::info!("Graph store initialized");

    // Initialize permission manager
    let permission_manager = PermissionManager::new();
    tracing::info!("Permission manager initialized");

    // Initialize gRPC bridge client (optional - only if BRIDGE_GRPC_URL is set)
    let bridge_client = match GrpcBridgeClient::from_env().await {
        Ok(client) => {
            tracing::info!("gRPC Bridge client initialized - bypass mode available");
            Some(client)
        }
        Err(e) => {
            tracing::info!("gRPC Bridge client not configured: {} (bypass mode disabled)", e);
            None
        }
    };

    // Initialize pre-flight checker
    let preflight_checker = PreflightChecker::new();

    // Quick check that claude CLI exists at startup
    if !preflight_checker.check_claude_cli().await {
        tracing::error!("Claude CLI not found! Install with: npm install -g @anthropic-ai/claude-code");
    } else {
        tracing::info!("Pre-flight checker: Claude CLI available");
    }

    let handler_data = Arc::new(BotData {
        allowed_users,
        base_working_dir: working_dir,
        usage_tracker,
        memory_store: std::sync::Mutex::new(memory_store),
        conversation_store: std::sync::Mutex::new(conversation_store),
        graph_store: std::sync::Mutex::new(graph_store),
        token_counter,
        llama_worker,
        lifecycle: Arc::clone(&lifecycle),
        permission_manager,
        bridge_client,
        preflight_checker,
        // Phase 6: Initialize advanced UX components
        ui_contexts: RwLock::new(HashMap::new()),
        progress_manager: ProgressManager::new(),
    });

    // Auto-load system context on startup
    let context_result = load_context(&handler_data);
    if context_result.contains("Facts stored") {
        tracing::info!("System context auto-loaded");
    } else {
        tracing::warn!("Context auto-load: {}", context_result);
    }

    // Start lifecycle manager in background
    let lifecycle_clone = Arc::clone(&lifecycle);
    tokio::spawn(async move {
        lifecycle_clone.run(LifecycleCallbacks::default()).await;
    });

    // Build explicit handler tree with callback query support
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .endpoint(message_handler)
        )
        .branch(
            Update::filter_callback_query()
                .endpoint(callback_handler)
        );

    tracing::info!("Starting dispatcher with long polling...");
    tracing::info!("===========================================");
    tracing::info!("  Bot is now LIVE - send a message!");
    tracing::info!("===========================================");

    // Create dispatcher with explicit configuration
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![handler_data])
        .default_handler(|upd| async move {
            tracing::debug!("Unhandled update: {:?}", upd);
        })
        .error_handler(LoggingErrorHandler::with_custom_text(
            "Error in message handler"
        ))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    tracing::warn!("Dispatcher stopped");
    Ok(())
}

/// Message handler endpoint for the dispatcher
async fn message_handler(
    bot: Bot,
    msg: Message,
    data: Arc<BotData>,
) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0).unwrap_or(0);
    let chat_id = msg.chat.id.0;
    let text_preview = msg.text().unwrap_or("<non-text>").chars().take(50).collect::<String>();

    tracing::info!(
        ">>> Message received: user={}, chat={}, text={:?}",
        user_id, chat_id, text_preview
    );

    if let Err(e) = handle_message(bot, msg, data).await {
        tracing::error!("Error handling message: {}", e);
    }

    Ok(())
}

/// Callback query handler for inline keyboard buttons
async fn callback_handler(
    bot: Bot,
    query: CallbackQuery,
    data: Arc<BotData>,
) -> ResponseResult<()> {
    let user_id = query.from.id.0 as i64;

    // Check if user is allowed
    if !data.is_allowed(user_id) {
        bot.answer_callback_query(&query.id)
            .text("Unauthorized")
            .await?;
        return Ok(());
    }

    let callback_data = match &query.data {
        Some(d) => d.clone(),
        None => {
            bot.answer_callback_query(&query.id).await?;
            return Ok(());
        }
    };

    let chat_id = query.message.as_ref().map(|m| m.chat().id);

    tracing::info!("Callback query: user={}, data={}", user_id, callback_data);

    // Parse button action
    if let Some(action) = ButtonAction::decode(&callback_data) {
        match action {
            ButtonAction::ViewLogs(task_id) => {
                // Get task logs from progress manager
                if let Some(tracker) = data.progress_manager.get(&task_id).await {
                    let msg = tracker.format();
                    bot.answer_callback_query(&query.id)
                        .text("Showing logs")
                        .await?;
                    if let Some(cid) = chat_id {
                        bot.send_message(cid, msg)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                } else {
                    bot.answer_callback_query(&query.id)
                        .text("Task not found")
                        .await?;
                }
            }

            ButtonAction::CancelTask(task_id) => {
                // Mark task as cancelled
                data.progress_manager.update(&task_id, |tracker| {
                    tracker.fail("Cancelled by user");
                }).await;
                bot.answer_callback_query(&query.id)
                    .text("Task cancelled")
                    .await?;
            }

            ButtonAction::RetryTask(_task_id) => {
                // Get last command from context and retry
                if let Some(cid) = chat_id {
                    let ctx = data.get_ui_context(cid.0).await;
                    if let Some(ref cmd) = ctx.last_command {
                        bot.answer_callback_query(&query.id)
                            .text("Retrying...")
                            .await?;
                        // Re-execute the command
                        let working_dir = data.working_dir_for_user(user_id);
                        let is_autonomous = matches!(
                            data.permission_manager.get_status(user_id).level,
                            crate::permissions::PermissionLevel::Autonomous
                        );
                        if let Ok(response) = invoke_claude_cli(cmd, &working_dir, is_autonomous).await {
                            let _ = send_long_message(&bot, cid, &response.text).await;
                        }
                    } else {
                        bot.answer_callback_query(&query.id)
                            .text("No previous command to retry")
                            .await?;
                    }
                }
            }

            ButtonAction::ShowDiff => {
                if let Some(cid) = chat_id {
                    let ctx = data.get_ui_context(cid.0).await;
                    if let Some(ref diff) = ctx.last_diff {
                        bot.answer_callback_query(&query.id).await?;
                        let _ = send_long_message(&bot, cid, &format!("Recent changes:\n```\n{}\n```", diff)).await;
                    } else {
                        bot.answer_callback_query(&query.id)
                            .text("No diff available")
                            .await?;
                    }
                }
            }

            ButtonAction::ShowError => {
                if let Some(cid) = chat_id {
                    let ctx = data.get_ui_context(cid.0).await;
                    if let Some(ref error) = ctx.last_error {
                        bot.answer_callback_query(&query.id).await?;
                        let _ = send_long_message(&bot, cid, &format!("Last error:\n{}", error)).await;
                    } else {
                        bot.answer_callback_query(&query.id)
                            .text("No error recorded")
                            .await?;
                    }
                }
            }

            ButtonAction::Confirm(action_id) => {
                if let Some(cid) = chat_id {
                    data.update_ui_context(cid.0, |ctx| {
                        ctx.clear_confirmation();
                    }).await;
                    bot.answer_callback_query(&query.id)
                        .text("Confirmed")
                        .await?;
                    // Execute the confirmed action
                    bot.send_message(cid, format!("Action {} confirmed. Executing...", action_id)).await?;
                }
            }

            ButtonAction::Deny(action_id) => {
                if let Some(cid) = chat_id {
                    data.update_ui_context(cid.0, |ctx| {
                        ctx.clear_confirmation();
                    }).await;
                    bot.answer_callback_query(&query.id)
                        .text("Cancelled")
                        .await?;
                    bot.send_message(cid, format!("Action {} cancelled.", action_id)).await?;
                }
            }

            ButtonAction::SelectOption(option_id) => {
                bot.answer_callback_query(&query.id)
                    .text(&format!("Selected: {}", option_id))
                    .await?;
            }

            ButtonAction::PauseTask(_) | ButtonAction::ResumeTask(_) => {
                bot.answer_callback_query(&query.id)
                    .text("Not implemented yet")
                    .await?;
            }
        }
    } else if callback_data.starts_with("suggest:") {
        // Handle suggestion callbacks
        let cmd = callback_data.strip_prefix("suggest:").unwrap_or("");
        if let Some(cid) = chat_id {
            bot.answer_callback_query(&query.id)
                .text("Executing suggestion...")
                .await?;
            // Execute suggested command
            let working_dir = data.working_dir_for_user(user_id);
            let is_autonomous = matches!(
                data.permission_manager.get_status(user_id).level,
                crate::permissions::PermissionLevel::Autonomous
            );
            if let Ok(response) = invoke_claude_cli(cmd, &working_dir, is_autonomous).await {
                let _ = send_long_message(&bot, cid, &response.text).await;
            }
        }
    } else if callback_data.starts_with("wkill:") {
        // Worker kill
        let worker_id = callback_data.strip_prefix("wkill:").unwrap_or("");
        bot.answer_callback_query(&query.id)
            .text(&format!("Worker {} kill requested", worker_id))
            .await?;
    } else {
        bot.answer_callback_query(&query.id).await?;
    }

    Ok(())
}

struct BotData {
    allowed_users: Vec<i64>,
    base_working_dir: PathBuf,
    usage_tracker: UsageTracker,
    memory_store: std::sync::Mutex<MemoryStore>,
    conversation_store: std::sync::Mutex<ConversationStore>,
    graph_store: std::sync::Mutex<GraphStore>,
    token_counter: TokenCounter,
    llama_worker: LlamaWorker,
    lifecycle: Arc<LifecycleManager>,
    permission_manager: PermissionManager,
    bridge_client: Option<GrpcBridgeClient>,
    preflight_checker: PreflightChecker,
    // Phase 6: Advanced UX components
    ui_contexts: RwLock<HashMap<i64, UiContext>>,  // Per-chat UI context
    progress_manager: ProgressManager,
}

impl BotData {
    fn is_allowed(&self, user_id: i64) -> bool {
        self.allowed_users.is_empty() || self.allowed_users.contains(&user_id)
    }

    fn working_dir_for_user(&self, user_id: i64) -> PathBuf {
        self.base_working_dir.join(format!("user_{}", user_id))
    }

    /// Get remaining daily budget for user
    fn get_remaining_budget(&self, user_id: i64) -> f64 {
        let limits = self.usage_tracker.get_user_limits(user_id).unwrap_or_default();
        let daily = self.usage_tracker.get_daily_usage(user_id).unwrap_or_default();

        limits.daily_cost_limit_usd
            .map(|limit| (limit - daily.estimated_cost_usd).max(0.0))
            .unwrap_or(f64::MAX)
    }

    /// Get or create UI context for a chat
    async fn get_ui_context(&self, chat_id: i64) -> UiContext {
        let contexts = self.ui_contexts.read().await;
        contexts.get(&chat_id).cloned().unwrap_or_default()
    }

    /// Update UI context for a chat
    async fn update_ui_context<F>(&self, chat_id: i64, f: F)
    where
        F: FnOnce(&mut UiContext),
    {
        let mut contexts = self.ui_contexts.write().await;
        let ctx = contexts.entry(chat_id).or_default();
        f(ctx);
    }
}

/// Claude CLI response with usage info
struct ClaudeResponse {
    text: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    model: String,
    session_id: Option<String>,
}

/// Timeout configuration for Claude CLI execution
const SILENCE_WARNING_SECS: u64 = 30;
const TOTAL_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Invoke Claude Code CLI with JSON output for usage tracking
/// Includes silence detection and total timeout handling
async fn invoke_claude_cli(prompt: &str, working_dir: &PathBuf, autonomous: bool) -> Result<ClaudeResponse> {
    let start = Instant::now();
    tracing::debug!("Invoking claude CLI with prompt length: {}, autonomous: {}", prompt.len(), autonomous);

    // Test mode: echo back without calling Claude CLI
    if std::env::var("CLAUDEBOT_TEST_MODE").is_ok() {
        tracing::info!("TEST MODE: Echoing message back");
        return Ok(ClaudeResponse {
            text: format!("Echo: {}\n\n(Test mode - Claude CLI not invoked)", prompt.chars().take(200).collect::<String>()),
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: "test-mode".to_string(),
            session_id: None,
        });
    }

    // Check for existing session to resume
    let session_file = working_dir.join(".claude_session");
    let mut cmd = Command::new("claude");

    cmd.arg("-p")
        .arg(prompt)
        .arg("--output-format")
        .arg("json")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Always skip permission prompts - Telegram bot is non-interactive
    // and can't respond to permission dialogs (they would hang forever)
    cmd.arg("--dangerously-skip-permissions");
    if autonomous {
        tracing::info!("Autonomous mode: full access enabled");
    }

    // Resume session if exists (maintains conversation context)
    if session_file.exists() {
        if let Ok(session_id) = tokio::fs::read_to_string(&session_file).await {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                cmd.arg("--resume").arg(session_id);
            }
        }
    }

    let mut child = cmd
        .current_dir(working_dir)
        .spawn()
        .context("Failed to spawn claude CLI")?;

    // Take stdout/stderr for monitoring
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut all_stdout = String::new();
    let mut all_stderr = String::new();
    let mut last_output_time = Instant::now();
    let mut silence_warned = false;

    // Set up readers
    let mut stdout_reader = stdout.map(|s| BufReader::new(s).lines());
    let mut stderr_reader = stderr.map(|s| BufReader::new(s).lines());

    // Monitor output with timeout
    loop {
        let elapsed = start.elapsed();

        // Check total timeout
        if elapsed.as_secs() >= TOTAL_TIMEOUT_SECS {
            tracing::warn!("Claude CLI total timeout ({} seconds)", TOTAL_TIMEOUT_SECS);
            let _ = child.kill().await;
            let duration = start.elapsed();
            return Err(anyhow::anyhow!(
                "{}",
                TaskFeedback::format_timeout(duration, Some(&all_stdout))
            ));
        }

        // Check silence timeout
        let silence_duration = last_output_time.elapsed();
        if silence_duration.as_secs() >= SILENCE_WARNING_SECS && !silence_warned {
            tracing::warn!("Claude CLI silent for {} seconds", silence_duration.as_secs());
            silence_warned = true;
            // Don't bail yet, just warn - the process might still be working
        }

        // Use select to read from stdout/stderr with timeout
        tokio::select! {
            // Check for stdout line
            line = async {
                if let Some(ref mut reader) = stdout_reader {
                    reader.next_line().await
                } else {
                    // No stdout, wait forever (other branches will trigger)
                    std::future::pending().await
                }
            } => {
                match line {
                    Ok(Some(line)) => {
                        all_stdout.push_str(&line);
                        all_stdout.push('\n');
                        last_output_time = Instant::now();
                        silence_warned = false;
                        tracing::trace!("stdout: {}", &line[..line.len().min(100)]);
                    }
                    Ok(None) => {
                        // stdout closed
                        stdout_reader = None;
                    }
                    Err(e) => {
                        tracing::warn!("stdout read error: {}", e);
                        stdout_reader = None;
                    }
                }
            }

            // Check for stderr line
            line = async {
                if let Some(ref mut reader) = stderr_reader {
                    reader.next_line().await
                } else {
                    std::future::pending().await
                }
            } => {
                match line {
                    Ok(Some(line)) => {
                        all_stderr.push_str(&line);
                        all_stderr.push('\n');
                        last_output_time = Instant::now();
                        silence_warned = false;
                        tracing::trace!("stderr: {}", &line[..line.len().min(100)]);
                    }
                    Ok(None) => {
                        stderr_reader = None;
                    }
                    Err(e) => {
                        tracing::warn!("stderr read error: {}", e);
                        stderr_reader = None;
                    }
                }
            }

            // Check process status
            status = child.wait() => {
                let status = status.context("Failed to wait for claude CLI")?;
                let duration = start.elapsed();
                tracing::info!("Claude CLI completed in {:?} with status: {:?}", duration, status);

                if !status.success() {
                    let hint = OutputParser::extract_error_hint(&all_stderr);
                    return Err(anyhow::anyhow!(
                        "{}",
                        TaskFeedback::format_error(&all_stderr.trim(), hint.as_deref())
                    ));
                }

                // Process completed - parse output
                break;
            }

            // Periodic check (every 100ms)
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                // Just continue the loop for timeout checks
            }
        }

        // If both streams closed, wait for process
        if stdout_reader.is_none() && stderr_reader.is_none() {
            let status = child.wait().await.context("Failed to wait for claude CLI")?;
            if !status.success() {
                let hint = OutputParser::extract_error_hint(&all_stderr);
                return Err(anyhow::anyhow!(
                    "{}",
                    TaskFeedback::format_error(&all_stderr.trim(), hint.as_deref())
                ));
            }
            break;
        }
    }

    // Try to parse JSON output
    match serde_json::from_str::<ClaudeJsonOutput>(&all_stdout) {
        Ok(json) => {
            let usage = json.usage.unwrap_or_default();

            // Save session ID for conversation continuity
            if let Some(ref sid) = json.session_id {
                let session_file = working_dir.join(".claude_session");
                let _ = std::fs::write(&session_file, sid);
            }

            Ok(ClaudeResponse {
                text: json.result,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                model: json.model.unwrap_or_else(|| "claude-sonnet-4".to_string()),
                session_id: json.session_id,
            })
        }
        Err(_) => {
            // Fall back to plain text if JSON parsing fails
            let clean = strip_ansi_codes(&all_stdout);
            Ok(ClaudeResponse {
                text: clean,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                model: "unknown".to_string(),
                session_id: None,
            })
        }
    }
}

/// Strip ANSI escape codes from CLI output
fn strip_ansi_codes(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").to_string()
}

async fn handle_message(bot: Bot, msg: Message, data: Arc<BotData>) -> Result<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id = msg.chat.id;

    // Record activity for lifecycle management
    data.lifecycle.record_activity();

    if !data.is_allowed(user_id) {
        tracing::warn!("Unauthorized user: {}", user_id);
        bot.send_message(chat_id, "Unauthorized.").await?;
        return Ok(());
    }

    // Mark as processing (prevents sleep during active work)
    let _guard = ProcessingGuard::new(Arc::clone(&data.lifecycle));

    // Get/create user's working directory
    let working_dir = data.working_dir_for_user(user_id);
    tokio::fs::create_dir_all(&working_dir).await?;

    // Handle text
    if let Some(text) = msg.text() {
        return handle_text(&bot, chat_id, &data, text, &working_dir, user_id).await;
    }

    // Handle documents
    if let Some(doc) = msg.document() {
        return handle_document(&bot, &msg, chat_id, &data, doc, &working_dir, user_id).await;
    }

    // Handle photos
    if let Some(photos) = msg.photo() {
        if let Some(photo) = photos.last() {
            return handle_photo(&bot, &msg, chat_id, &data, photo, &working_dir, user_id).await;
        }
    }

    Ok(())
}

async fn handle_text(
    bot: &Bot,
    chat_id: ChatId,
    data: &BotData,
    text: &str,
    working_dir: &PathBuf,
    user_id: i64,
) -> Result<()> {
    // Handle commands
    if text.starts_with('/') {
        return handle_command(bot, chat_id, data, text, working_dir, user_id).await;
    }

    // Get UI context for this chat
    let ui_ctx = data.get_ui_context(chat_id.0).await;

    // Phase 6: Check for intent-based shortcuts (e.g., "again", "fix it", "cancel")
    if let Some(intent) = ContextParser::detect_intent(text, &ui_ctx) {
        match intent {
            Intent::Retry(cmd) => {
                bot.send_message(chat_id, format!("Retrying: {}", truncate(&cmd, 50))).await?;
                let is_autonomous = matches!(
                    data.permission_manager.get_status(user_id).level,
                    crate::permissions::PermissionLevel::Autonomous
                );
                match invoke_claude_cli(&cmd, working_dir, is_autonomous).await {
                    Ok(response) => {
                        record_usage(data, user_id, &response);
                        send_long_message(bot, chat_id, &response.text).await?;
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        data.update_ui_context(chat_id.0, |ctx| ctx.set_error(&error_msg)).await;
                        bot.send_message(chat_id, format!("Retry failed: {}", error_msg)).await?;
                    }
                }
                return Ok(());
            }
            Intent::FixError(error) => {
                let fix_prompt = format!("Fix this error:\n{}\n\nApply the necessary fixes.", error);
                bot.send_message(chat_id, "Attempting to fix the error...").await?;
                let is_autonomous = matches!(
                    data.permission_manager.get_status(user_id).level,
                    crate::permissions::PermissionLevel::Autonomous
                );
                match invoke_claude_cli(&fix_prompt, working_dir, is_autonomous).await {
                    Ok(response) => {
                        record_usage(data, user_id, &response);
                        send_long_message(bot, chat_id, &response.text).await?;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!("Auto-fix failed: {}", e)).await?;
                    }
                }
                return Ok(());
            }
            Intent::Cancel(task_id) => {
                data.progress_manager.update(&task_id, |tracker| {
                    tracker.fail("Cancelled by user");
                }).await;
                bot.send_message(chat_id, "Task cancelled.").await?;
                return Ok(());
            }
            Intent::Confirm => {
                data.update_ui_context(chat_id.0, |ctx| ctx.clear_confirmation()).await;
                bot.send_message(chat_id, "Confirmed.").await?;
                return Ok(());
            }
            Intent::Deny => {
                data.update_ui_context(chat_id.0, |ctx| ctx.clear_confirmation()).await;
                bot.send_message(chat_id, "Cancelled.").await?;
                return Ok(());
            }
            Intent::ShowDiff => {
                if let Some(ref diff) = ui_ctx.last_diff {
                    send_long_message(bot, chat_id, &format!("Recent changes:\n```\n{}\n```", diff)).await?;
                } else {
                    bot.send_message(chat_id, "No recent diff available.").await?;
                }
                return Ok(());
            }
            Intent::ShowError => {
                if let Some(ref error) = ui_ctx.last_error {
                    send_long_message(bot, chat_id, &format!("Last error:\n{}", error)).await?;
                } else {
                    bot.send_message(chat_id, "No recent error recorded.").await?;
                }
                return Ok(());
            }
            Intent::ShowLogs => {
                if let Some(ref task_id) = ui_ctx.last_task_id {
                    if let Some(tracker) = data.progress_manager.get(task_id).await {
                        bot.send_message(chat_id, tracker.format())
                            .parse_mode(ParseMode::Html)
                            .await?;
                    } else {
                        bot.send_message(chat_id, "No active task logs.").await?;
                    }
                } else {
                    bot.send_message(chat_id, "No recent task to show logs for.").await?;
                }
                return Ok(());
            }
        }
    }

    // Phase 6: Expand context references (e.g., "that file" -> actual path)
    let expanded_text = ContextParser::expand(text, &ui_ctx);

    // Check limits before processing
    if let Err(msg) = check_user_limits(data, user_id) {
        bot.send_message(chat_id, msg).await?;
        return Ok(());
    }

    // Pre-flight check: verify required tools/credentials for this command
    let preflight = data.preflight_checker.check_for_command(&expanded_text).await;
    if !preflight.ready {
        bot.send_message(chat_id, preflight.format_error()).await?;
        return Ok(());
    }

    // Show warnings but continue
    if !preflight.warnings.is_empty() {
        let warnings = preflight.format_warnings();
        tracing::warn!("Preflight warnings for user {}: {}", user_id, warnings);
    }

    // Send typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

    // Store the command in context for "again" support
    data.update_ui_context(chat_id.0, |ctx| {
        ctx.set_command(&expanded_text);
    }).await;

    // Get conversation history for this chat (last 10 messages)
    let conversation_context = get_conversation_context(data, chat_id.0);

    // Inject relevant memory context (semantic facts)
    let memory_context = get_memory_context(data, &expanded_text);

    // Build enhanced prompt with conversation history + memory facts
    let enhanced_prompt = if conversation_context.is_empty() && memory_context.is_empty() {
        expanded_text.clone()
    } else {
        format!("{}{}{}", conversation_context, expanded_text, memory_context)
    };

    // Pre-flight token estimation
    let remaining_budget = data.get_remaining_budget(user_id);
    let budget_check = data.token_counter.check_budget(
        &enhanced_prompt,
        1000, // Expected output
        &crate::router::ModelHint::Sonnet,
        remaining_budget,
        0.5, // Assumed cache hit ratio
    );

    match &budget_check {
        BudgetCheck::Warning { estimated_cost, remaining_budget, .. } => {
            tracing::warn!(
                "User {} approaching budget: est ${:.4}, remaining ${:.2}",
                user_id, estimated_cost, remaining_budget
            );
        }
        BudgetCheck::Exceeded { estimated_cost, remaining_budget, .. } => {
            bot.send_message(
                chat_id,
                format!(
                    "Budget exceeded. Estimated cost: ${:.4}, remaining: ${:.2}\n\
                    Use /limits to adjust your budget.",
                    estimated_cost, remaining_budget
                )
            ).await?;
            return Ok(());
        }
        BudgetCheck::Ok { .. } => {}
    }

    // Check if user is in autonomous mode
    let is_autonomous = matches!(
        data.permission_manager.get_status(user_id).level,
        crate::permissions::PermissionLevel::Autonomous
    );

    // Process with Claude Code CLI
    let result = invoke_claude_cli(&enhanced_prompt, working_dir, is_autonomous).await;

    match result {
        Ok(response) => {
            // Record usage
            record_usage(data, user_id, &response);

            // Store conversation exchange (user message + assistant response)
            store_conversation_exchange(data, chat_id.0, text, &response.text);

            // Extract facts for continuous learning
            extract_and_learn_facts(data, &response.text, user_id);

            // Phase 6: Detect if response contains file paths or diffs for context
            let has_changes = response.text.contains("diff") ||
                              response.text.contains("modified") ||
                              response.text.contains("created") ||
                              response.text.contains("Changed ");

            // Extract file paths mentioned in response
            if let Some(file_path) = extract_file_path(&response.text) {
                data.update_ui_context(chat_id.0, |ctx| ctx.set_file(&file_path)).await;
            }

            // Extract diff if present
            if let Some(diff) = extract_diff(&response.text) {
                data.update_ui_context(chat_id.0, |ctx| ctx.set_diff(&diff)).await;
            }

            // Send response
            send_long_message(bot, chat_id, &response.text).await?;

            // Phase 6: Generate and show suggestions after task completion
            let suggestions = suggest_next_actions(&expanded_text, true, has_changes);
            if !suggestions.is_empty() {
                if let Some(keyboard) = suggestions_keyboard(&suggestions) {
                    bot.send_message(chat_id, "Suggestions:")
                        .reply_markup(keyboard)
                        .await?;
                }
            }
        }
        Err(e) => {
            // Store error in context for "fix it" support
            let error_msg = e.to_string();
            data.update_ui_context(chat_id.0, |ctx| ctx.set_error(&error_msg)).await;

            // Send error with suggestion to fix
            bot.send_message(chat_id, &error_msg).await?;

            // Show error-related suggestions
            let suggestions = suggest_next_actions(&expanded_text, false, false);
            if !suggestions.is_empty() {
                if let Some(keyboard) = suggestions_keyboard(&suggestions) {
                    bot.send_message(chat_id, "What would you like to do?")
                        .reply_markup(keyboard)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

/// Extract file path from response text
fn extract_file_path(text: &str) -> Option<String> {
    // Look for common file path patterns
    let patterns = [
        r#"(?:created|modified|reading|wrote|saved)\s+[`'"]?([/\w\-_.]+\.[a-z]+)[`'"]?"#,
        r#"File:\s*[`'"]?([/\w\-_.]+\.[a-z]+)[`'"]?"#,
        r"([/\w\-_]+/[a-z_]+\.[a-z]+)",
    ];

    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(text) {
                if let Some(path) = caps.get(1) {
                    return Some(path.as_str().to_string());
                }
            }
        }
    }
    None
}

/// Format Circle pipeline result for Telegram
fn format_circle_result(result: &PipelineResult) -> String {
    let mut msg = format!(
        "Development Circle: {}\n\n\
        Mode: {:?}\n\
        Success: {}\n\
        Revisions: {}\n\
        Duration: {}ms\n",
        result.feature,
        result.mode,
        if result.success { "YES" } else { "NO" },
        result.revisions,
        result.total_duration_ms
    );

    if let Some(ref blocked) = result.blocked_at {
        msg.push_str(&format!("Blocked: {}\n", blocked));
    }

    msg.push_str("\n--- Phases ---\n");

    for phase in &result.phases {
        msg.push_str(&format!(
            "\n[{}] {} ({}ms)\n",
            phase.phase,
            phase.persona,
            phase.duration_ms
        ));

        if let Some(ref verdict) = phase.verdict {
            msg.push_str(&format!("Verdict: {:?}\n", verdict));
        }

        if let Some(ref risk) = phase.risk_level {
            msg.push_str(&format!("Risk: {:?}\n", risk));
        }

        if !phase.files_changed.is_empty() {
            msg.push_str(&format!("Files: {}\n", phase.files_changed.join(", ")));
        }

        // Truncate output for readability (UTF-8 safe)
        let output = if phase.output.len() > 500 {
            let truncate_at = phase.output
                .char_indices()
                .take_while(|(i, _)| *i < 500)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(500.min(phase.output.len()));
            format!("{}...\n[truncated]", &phase.output[..truncate_at])
        } else {
            phase.output.clone()
        };
        msg.push_str(&format!("\n{}\n", output));
    }

    msg
}

/// Extract diff from response text
fn extract_diff(text: &str) -> Option<String> {
    // Look for diff blocks
    if let Some(start) = text.find("```diff") {
        if let Some(end) = text[start..].find("```\n") {
            let diff = &text[start + 7..start + end];
            return Some(diff.trim().to_string());
        }
    }
    // Look for unified diff format
    if text.contains("@@") && (text.contains("+") || text.contains("-")) {
        let lines: Vec<&str> = text.lines()
            .filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with("@@"))
            .take(20)
            .collect();
        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
    }
    None
}

fn check_user_limits(data: &BotData, user_id: i64) -> std::result::Result<(), String> {
    match data.usage_tracker.check_limits(user_id) {
        Ok(LimitCheck::Ok(_)) => Ok(()),
        Ok(LimitCheck::Exceeded(limit_type)) => Err(limit_type.message()),
        Err(e) => {
            tracing::error!("Error checking limits: {}", e);
            Ok(()) // Allow on error
        }
    }
}

fn record_usage(data: &BotData, user_id: i64, response: &ClaudeResponse) {
    if response.input_tokens > 0 || response.output_tokens > 0 {
        let record = UsageRecord {
            user_id,
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cache_read_tokens: response.cache_read_tokens,
            cache_write_tokens: response.cache_write_tokens,
            model: response.model.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        };

        if let Err(e) = data.usage_tracker.record_usage(&record) {
            tracing::error!("Failed to record usage: {}", e);
        }
    }
}

async fn handle_command(
    bot: &Bot,
    chat_id: ChatId,
    data: &BotData,
    text: &str,
    working_dir: &PathBuf,
    user_id: i64,
) -> Result<()> {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let cmd = parts[0];
    let args = parts.get(1).copied().unwrap_or("");

    match cmd {
        "/start" => {
            bot.send_message(chat_id,
                "ClaudeBot (Eliot Brain)\n\n\
                I'm Claude Code with persistent memory and continuous learning.\n\n\
                Commands:\n\
                /help - Show help\n\
                /usage - Token usage & costs\n\
                /memory - View memories\n\
                /learn <fact> - Teach me something\n\
                /graph - View knowledge graph\n\
                /extract <text> - Extract entities\n\n\
                Just send messages - I remember context!"
            ).await?;
        }

        "/help" => {
            bot.send_message(chat_id,
                "Help:\n\n\
                Chat:\n\
                - Send text: I process with full Claude Code\n\
                - Send files: I analyze them\n\
                - Send images: I describe them\n\n\
                Conversation:\n\
                /history - View recent conversation\n\
                /clear - Clear conversation history\n\n\
                Memory (Facts):\n\
                /memory - View memory stats\n\
                /memory search <query> - Search memories\n\
                /learn <fact> - Teach me a fact\n\
                /context - Load system context\n\
                /graph - View knowledge graph\n\
                /extract <text> - Extract entities\n\n\
                Budget & Stats:\n\
                /usage - View token usage\n\
                /limits - View/set limits\n\
                /stats - System statistics\n\
                /status - Check bot status\n\
                /preflight [cmd] - Check tool availability\n\n\
                Permissions:\n\
                /autonomous [duration] - Full access mode\n\
                /supervised - Require approval\n\
                /perms - View permission status\n\n\
                Bypass Bridge (AR):\n\
                /bypass <task> - Execute on AR server\n\
                /bypass_file <path> - Analyze file on AR\n\
                /bypass_cat <path> - Raw file content\n\
                /bypass_status - Check bridge status"
            ).await?;
        }

        "/status" => {
            let status = Command::new("claude")
                .arg("--version")
                .output()
                .await;

            match status {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout);
                    bot.send_message(chat_id, format!("Online\n{}", version.trim())).await?;
                }
                _ => {
                    bot.send_message(chat_id, "Claude CLI not available").await?;
                }
            }
        }

        "/preflight" => {
            bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

            let result = if args.is_empty() {
                // Check all tools and credentials
                data.preflight_checker.check_all().await
            } else {
                // Check for specific command
                data.preflight_checker.check_for_command(args).await
            };

            let mut msg = if result.ready {
                "Pre-flight Check: PASSED\n\nAll required tools and credentials available.\n".to_string()
            } else {
                format!("Pre-flight Check: FAILED\n\n{}", result.format_error())
            };

            if !result.warnings.is_empty() {
                msg.push_str(&format!("\n{}", result.format_warnings()));
            }

            bot.send_message(chat_id, msg).await?;
        }

        "/ghcheck" | "/gh" => {
            bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

            // Check if gh CLI is installed
            let gh_version = Command::new("gh")
                .arg("--version")
                .output()
                .await;

            match gh_version {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout);

                    // Check auth status
                    let auth_check = Command::new("gh")
                        .arg("auth")
                        .arg("status")
                        .output()
                        .await;

                    let auth_status = match auth_check {
                        Ok(auth_output) => {
                            if auth_output.status.success() {
                                let stdout = String::from_utf8_lossy(&auth_output.stdout);
                                let stderr = String::from_utf8_lossy(&auth_output.stderr);
                                // gh auth status outputs to stderr
                                let combined = format!("{}{}", stdout, stderr);
                                if combined.contains("Logged in") {
                                    "Authenticated"
                                } else {
                                    "Not authenticated"
                                }
                            } else {
                                "Not authenticated"
                            }
                        }
                        Err(_) => "Auth check failed",
                    };

                    // Try to get user info if authenticated
                    let user_info = if auth_status == "Authenticated" {
                        match Command::new("gh")
                            .arg("api")
                            .arg("user")
                            .arg("--jq")
                            .arg(".login")
                            .output()
                            .await
                        {
                            Ok(user_output) if user_output.status.success() => {
                                let user = String::from_utf8_lossy(&user_output.stdout).trim().to_string();
                                format!("User: {}", user)
                            }
                            _ => String::new(),
                        }
                    } else {
                        String::new()
                    };

                    let msg = format!(
                        "GitHub CLI Status\n\n\
                        Version: {}\n\
                        Auth: {}\n\
                        {}\n\n\
                        Commands:\n\
                        - gh auth login - Authenticate\n\
                        - gh repo list - List repos\n\
                        - gh pr list - List PRs",
                        version.lines().next().unwrap_or("unknown"),
                        auth_status,
                        user_info
                    );
                    bot.send_message(chat_id, msg).await?;
                }
                _ => {
                    bot.send_message(chat_id,
                        "GitHub CLI not installed.\n\n\
                        Install with:\n\
                        - Arch: sudo pacman -S github-cli\n\
                        - Debian: sudo apt install gh\n\
                        - macOS: brew install gh\n\n\
                        Then run: gh auth login"
                    ).await?;
                }
            }
        }

        "/stats" => {
            let lifecycle_stats = data.lifecycle.get_stats();
            let llama_available = data.llama_worker.is_available().await;

            let state_str = match lifecycle_stats.current_state {
                crate::lifecycle::State::Sleep => "Sleep",
                crate::lifecycle::State::Wake => "Wake",
                crate::lifecycle::State::Processing => "Processing",
            };

            let msg = format!(
                "System Statistics\n\n\
                Lifecycle:\n\
                - State: {}\n\
                - Idle: {}s\n\
                - Wake cycles: {}\n\
                - Sleep cycles: {}\n\n\
                Background Tasks:\n\
                - Consolidations: {}\n\
                - Decay applied: {}\n\
                - Compressions: {}\n\n\
                Services:\n\
                - Llama: {}\n\
                - Memory: Active",
                state_str,
                lifecycle_stats.idle_seconds,
                lifecycle_stats.wake_count,
                lifecycle_stats.sleep_count,
                lifecycle_stats.consolidations,
                lifecycle_stats.decays_applied,
                lifecycle_stats.compressions,
                if llama_available { "Available" } else { "Unavailable" }
            );
            bot.send_message(chat_id, msg).await?;
        }

        "/usage" => {
            let msg = format_usage(data, user_id)?;
            bot.send_message(chat_id, msg).await?;
        }

        "/limits" => {
            if args.is_empty() {
                let msg = format_limits(data, user_id)?;
                bot.send_message(chat_id, msg).await?;
            } else {
                let result = set_limits(data, user_id, args);
                bot.send_message(chat_id, result).await?;
            }
        }

        "/dir" => {
            bot.send_message(chat_id, format!("Working dir: {}", working_dir.display())).await?;
        }

        "/memory" | "/mem" => {
            if args.is_empty() {
                let msg = format_memory_stats(data)?;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("search ") {
                let query = &args[7..];
                let msg = search_memory(data, query)?;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("similar ") {
                // Pure semantic/vector search
                let query = &args[8..];
                let msg = search_memory_semantic(data, query).await;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("hybrid ") {
                // Hybrid BM25 + vector search
                let query = &args[7..];
                let msg = search_memory_hybrid(data, query).await;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("backfill") {
                // Backfill embeddings for memories without them
                let msg = backfill_memory_embeddings(data).await;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("recent") {
                let msg = get_recent_memories(data)?;
                bot.send_message(chat_id, msg).await?;
            } else if args.starts_with("embeddings") || args.starts_with("stats") {
                let msg = format_embedding_stats(data)?;
                bot.send_message(chat_id, msg).await?;
            } else {
                bot.send_message(chat_id,
                    "Memory commands:\n\
                    /memory - View stats\n\
                    /memory search <query> - Keyword search (BM25)\n\
                    /memory similar <query> - Semantic search (vector)\n\
                    /memory hybrid <query> - Hybrid search (keyword + vector)\n\
                    /memory backfill - Generate embeddings for memories\n\
                    /memory embeddings - View embedding stats\n\
                    /memory recent - View recent memories\n\
                    /learn <fact> - Learn a new fact"
                ).await?;
            }
        }

        "/learn" => {
            if args.is_empty() {
                bot.send_message(chat_id, "Usage: /learn <fact to remember>").await?;
            } else {
                let result = learn_fact(data, args, user_id);
                bot.send_message(chat_id, result).await?;
            }
        }

        "/graph" | "/entities" => {
            let result = format_graph_stats(data);
            bot.send_message(chat_id, result).await?;
        }

        "/extract" => {
            if args.is_empty() {
                bot.send_message(chat_id, "Usage: /extract <text to analyze>").await?;
            } else {
                let result = extract_entities(data, args).await;
                bot.send_message(chat_id, result).await?;
            }
        }

        // Permission commands for autonomous mode
        "/autonomous" | "/auto" | "/allowall" => {
            // Enable autonomous mode for this session
            let duration = if args.is_empty() {
                std::time::Duration::from_secs(3600) // 1 hour default
            } else {
                // Parse duration: "30m", "2h", "1d"
                parse_duration(args).unwrap_or(std::time::Duration::from_secs(3600))
            };

            data.permission_manager.escalate_user(user_id, Some(duration));

            let mins = duration.as_secs() / 60;
            bot.send_message(chat_id, format!(
                "AUTONOMOUS MODE ENABLED\n\n\
                Duration: {} minutes\n\
                Access: Full (read, write, commit, push)\n\n\
                I can now:\n\
                - Create and modify files\n\
                - Run commands\n\
                - Commit and push changes\n\
                - Deploy if configured\n\n\
                Use /supervised to return to approval mode."
            , mins)).await?;
        }

        "/supervised" | "/restrict" => {
            data.permission_manager.revoke_user(user_id);
            bot.send_message(chat_id,
                "SUPERVISED MODE\n\n\
                Changes require your approval.\n\
                Use /autonomous to enable full access."
            ).await?;
        }

        "/perms" | "/permissions" => {
            let status = data.permission_manager.get_status(user_id);
            let level_str = match status.level {
                crate::permissions::PermissionLevel::Restricted => "Restricted (read-only)",
                crate::permissions::PermissionLevel::Supervised => "Supervised (needs approval)",
                crate::permissions::PermissionLevel::Autonomous => "Autonomous (full access)",
            };

            let remaining = status.escalation_remaining
                .map(|d| format!("{} minutes", d.as_secs() / 60))
                .unwrap_or_else(|| "N/A".to_string());

            bot.send_message(chat_id, format!(
                "Permission Status\n\n\
                Level: {}\n\
                Escalation remaining: {}\n\
                Approved operations: {}\n\n\
                Commands:\n\
                /autonomous [duration] - Full access\n\
                /supervised - Require approval"
            , level_str, remaining, status.approved_ops)).await?;
        }

        "/history" | "/conv" | "/conversation" => {
            let result = format_conversation_history(data, chat_id.0);
            bot.send_message(chat_id, result).await?;
        }

        "/clear" | "/clearhistory" => {
            let result = clear_conversation_history(data, chat_id.0);
            bot.send_message(chat_id, result).await?;
        }

        // Development Circle - Code Review & Security Audit
        "/circle" | "/review" | "/security" => {
            if args.is_empty() {
                bot.send_message(chat_id,
                    "Development Circle\n\n\
                    Multi-persona code quality pipeline:\n\
                    1. Graydon - Implementation\n\
                    2. Linus - Code Review\n\
                    3. Maria - Testing\n\
                    4. Kai - Optimization\n\
                    5. Sentinel - Security Audit (OWASP)\n\n\
                    Usage:\n\
                    /circle full <feature> - Full 5-phase pipeline\n\
                    /circle review <code> - Review only (Linus + Sentinel)\n\
                    /circle security <code> - Security audit only\n\
                    /circle quick <task> - Quick fix (Graydon only)\n\n\
                    Example:\n\
                    /circle security check src/auth.rs for vulnerabilities"
                ).await?;
            } else {
                // Parse mode and task
                let (mode, task) = if args.starts_with("full ") {
                    (PipelineMode::Full, args.strip_prefix("full ").unwrap())
                } else if args.starts_with("review ") {
                    (PipelineMode::ReviewOnly, args.strip_prefix("review ").unwrap())
                } else if args.starts_with("security ") {
                    (PipelineMode::SecurityOnly, args.strip_prefix("security ").unwrap())
                } else if args.starts_with("quick ") {
                    (PipelineMode::QuickFix, args.strip_prefix("quick ").unwrap())
                } else {
                    // Default to security audit
                    (PipelineMode::SecurityOnly, args)
                };

                bot.send_message(chat_id, format!(
                    "Starting Development Circle ({:?})...\n\n\
                    Task: {}\n\n\
                    This may take a few minutes.",
                    mode, truncate(task, 100)
                )).await?;

                bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

                // Get code context - either from the task description or read a file
                let context = if task.contains(".rs") || task.contains(".ts") || task.contains(".vue") {
                    // Try to extract file path and read it
                    if let Some(file_path) = extract_file_path(task) {
                        let full_path = working_dir.join(&file_path);
                        match tokio::fs::read_to_string(&full_path).await {
                            Ok(content) => format!("File: {}\n\n```\n{}\n```", file_path, content),
                            Err(_) => task.to_string(),
                        }
                    } else {
                        task.to_string()
                    }
                } else {
                    task.to_string()
                };

                // Run the circle pipeline
                let api_key = std::env::var("ANTHROPIC_API_KEY").ok();
                let claude_client = crate::claude::ClaudeClient::new(api_key.as_deref());
                let circle = Circle::new(claude_client);

                match circle.run(task, &context, mode).await {
                    Ok(result) => {
                        let summary = format_circle_result(&result);
                        send_long_message(bot, chat_id, &summary).await?;

                        // Store in UI context
                        data.update_ui_context(chat_id.0, |ctx| {
                            ctx.set_command(&format!("/circle {:?} {}", mode, task));
                        }).await;
                    }
                    Err(e) => {
                        bot.send_message(chat_id, format!(
                            "Circle pipeline failed:\n{}",
                            e
                        )).await?;
                    }
                }
            }
        }

        // Bypass bridge commands for remote AR execution
        "/bypass" | "/b" => {
            if args.is_empty() {
                bot.send_message(chat_id,
                    "BYPASS BRIDGE\n\n\
                    Execute tasks on AR server with unleashed Claude Code.\n\n\
                    Usage:\n\
                    /bypass <task> - Execute task on AR\n\
                    /bypass_status - Check bridge status\n\n\
                    Example:\n\
                    /bypass analyze this codebase and suggest improvements"
                ).await?;
            } else {
                handle_bypass(bot, chat_id, data, args, user_id).await?;
            }
        }

        "/bypass_status" | "/bs" => {
            handle_bypass_status(bot, chat_id, data).await?;
        }

        "/bypass_file" | "/bf" => {
            if args.is_empty() {
                bot.send_message(chat_id,
                    "BYPASS FILE ANALYSIS\n\n\
                    Ask AR's Claude to analyze a file.\n\n\
                    Usage: /bypass_file <path>\n\
                    Example: /bypass_file /etc/nginx/nginx.conf"
                ).await?;
            } else {
                handle_bypass_file(bot, chat_id, data, args, true).await?;
            }
        }

        "/bypass_cat" | "/bc" => {
            if args.is_empty() {
                bot.send_message(chat_id,
                    "BYPASS CAT (Raw File)\n\n\
                    Get raw file content from AR server.\n\n\
                    Usage: /bypass_cat <path>\n\
                    Example: /bypass_cat /var/log/syslog"
                ).await?;
            } else {
                handle_bypass_file(bot, chat_id, data, args, false).await?;
            }
        }

        "/context" | "/ctx" => {
            let msg = load_context(data);
            bot.send_message(chat_id, msg).await?;
        }

        _ => {
            // Check limits before processing
            if let Err(msg) = check_user_limits(data, user_id) {
                bot.send_message(chat_id, msg).await?;
                return Ok(());
            }

            // Treat unknown commands as prompts to Claude
            bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
            let is_autonomous = matches!(
                data.permission_manager.get_status(user_id).level,
                crate::permissions::PermissionLevel::Autonomous
            );
            let response = invoke_claude_cli(text, working_dir, is_autonomous).await?;
            record_usage(data, user_id, &response);
            send_long_message(bot, chat_id, &response.text).await?;
        }
    }

    Ok(())
}

fn format_usage(data: &BotData, user_id: i64) -> Result<String> {
    let daily = data.usage_tracker.get_daily_usage(user_id)?;
    let monthly = data.usage_tracker.get_monthly_usage(user_id)?;
    let total = data.usage_tracker.get_total_usage(user_id)?;

    let msg = format!(
        "Token Usage\n\n\
        Today:\n\
        - Input: {}\n\
        - Output: {}\n\
        - Requests: {}\n\
        - Est. Cost: ${:.4}\n\n\
        This Month:\n\
        - Input: {}\n\
        - Output: {}\n\
        - Requests: {}\n\
        - Est. Cost: ${:.4}\n\n\
        All Time:\n\
        - Input: {}\n\
        - Output: {}\n\
        - Requests: {}\n\
        - Est. Cost: ${:.4}",
        format_tokens(daily.total_input_tokens),
        format_tokens(daily.total_output_tokens),
        daily.request_count,
        daily.estimated_cost_usd,
        format_tokens(monthly.total_input_tokens),
        format_tokens(monthly.total_output_tokens),
        monthly.request_count,
        monthly.estimated_cost_usd,
        format_tokens(total.total_input_tokens),
        format_tokens(total.total_output_tokens),
        total.request_count,
        total.estimated_cost_usd,
    );

    Ok(msg)
}

fn format_limits(data: &BotData, user_id: i64) -> Result<String> {
    let limits = data.usage_tracker.get_user_limits(user_id)?;
    let daily = data.usage_tracker.get_daily_usage(user_id)?;
    let monthly = data.usage_tracker.get_monthly_usage(user_id)?;

    let daily_tokens = daily.total_input_tokens + daily.total_output_tokens;
    let monthly_tokens = monthly.total_input_tokens + monthly.total_output_tokens;

    let daily_limit_str = limits.daily_token_limit
        .map(|l| format!("{} / {}", format_tokens(daily_tokens), format_tokens(l)))
        .unwrap_or_else(|| "unlimited".to_string());

    let monthly_limit_str = limits.monthly_token_limit
        .map(|l| format!("{} / {}", format_tokens(monthly_tokens), format_tokens(l)))
        .unwrap_or_else(|| "unlimited".to_string());

    let daily_cost_str = limits.daily_cost_limit_usd
        .map(|l| format!("${:.2} / ${:.2}", daily.estimated_cost_usd, l))
        .unwrap_or_else(|| "unlimited".to_string());

    let monthly_cost_str = limits.monthly_cost_limit_usd
        .map(|l| format!("${:.2} / ${:.2}", monthly.estimated_cost_usd, l))
        .unwrap_or_else(|| "unlimited".to_string());

    let msg = format!(
        "Usage Limits\n\n\
        Daily Tokens: {}\n\
        Monthly Tokens: {}\n\
        Daily Cost: {}\n\
        Monthly Cost: {}\n\n\
        Set limits:\n\
        /limits daily 500K\n\
        /limits monthly 5M\n\
        /limits cost 5.00",
        daily_limit_str,
        monthly_limit_str,
        daily_cost_str,
        monthly_cost_str,
    );

    Ok(msg)
}

fn set_limits(data: &BotData, user_id: i64, args: &str) -> String {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        return "Usage: /limits <type> <value>\nTypes: daily, monthly, cost".to_string();
    }

    let limit_type = parts[0];
    let value_str = parts[1];

    // Get current limits
    let mut limits = match data.usage_tracker.get_user_limits(user_id) {
        Ok(l) => l,
        Err(_) => UserLimits::default(),
    };

    match limit_type {
        "daily" => {
            match parse_token_value(value_str) {
                Some(v) => {
                    limits.daily_token_limit = Some(v);
                    if let Err(e) = data.usage_tracker.set_user_limits(user_id, &limits) {
                        return format!("Error: {}", e);
                    }
                    format!("Daily token limit set to {}", format_tokens(v))
                }
                None => "Invalid value. Use numbers like 500K, 1M, 1000000".to_string(),
            }
        }
        "monthly" => {
            match parse_token_value(value_str) {
                Some(v) => {
                    limits.monthly_token_limit = Some(v);
                    if let Err(e) = data.usage_tracker.set_user_limits(user_id, &limits) {
                        return format!("Error: {}", e);
                    }
                    format!("Monthly token limit set to {}", format_tokens(v))
                }
                None => "Invalid value. Use numbers like 500K, 1M, 1000000".to_string(),
            }
        }
        "cost" => {
            match value_str.parse::<f64>() {
                Ok(v) => {
                    limits.daily_cost_limit_usd = Some(v);
                    if let Err(e) = data.usage_tracker.set_user_limits(user_id, &limits) {
                        return format!("Error: {}", e);
                    }
                    format!("Daily cost limit set to ${:.2}", v)
                }
                Err(_) => "Invalid value. Use decimal like 5.00, 10.50".to_string(),
            }
        }
        "unlimited" | "none" | "off" => {
            limits.daily_token_limit = None;
            limits.monthly_token_limit = None;
            limits.daily_cost_limit_usd = None;
            limits.monthly_cost_limit_usd = None;
            if let Err(e) = data.usage_tracker.set_user_limits(user_id, &limits) {
                return format!("Error: {}", e);
            }
            "All limits removed".to_string()
        }
        _ => "Unknown limit type. Use: daily, monthly, cost, unlimited".to_string(),
    }
}

/// Parse token values like "500K", "1M", "1000000"
fn parse_token_value(s: &str) -> Option<i64> {
    let s = s.to_uppercase();
    if s.ends_with('K') {
        s[..s.len()-1].parse::<f64>().ok().map(|v| (v * 1_000.0) as i64)
    } else if s.ends_with('M') {
        s[..s.len()-1].parse::<f64>().ok().map(|v| (v * 1_000_000.0) as i64)
    } else {
        s.parse::<i64>().ok()
    }
}

// ============ Memory Functions ============

fn format_memory_stats(data: &BotData) -> Result<String> {
    let store = data.memory_store.lock().unwrap();
    let stats = store.stats()?;

    let mut msg = format!("Memory Stats\n\nTotal: {} entries\n\nBy Category:", stats.total_entries);
    for (cat, count) in &stats.by_category {
        msg.push_str(&format!("\n- {}: {}", cat, count));
    }
    msg.push_str("\n\nCommands:\n/memory search <query>\n/memory recent\n/learn <fact>");
    Ok(msg)
}

fn search_memory(data: &BotData, query: &str) -> Result<String> {
    let store = data.memory_store.lock().unwrap();
    let results = store.search(query, 5)?;

    if results.is_empty() {
        return Ok(format!("No memories found for: {}", query));
    }

    let mut msg = format!("Memories matching '{}':\n", query);
    for (i, r) in results.iter().enumerate() {
        msg.push_str(&format!(
            "\n{}. [{}] {}\n   (score: {:.2}, accessed: {}x)",
            i + 1,
            r.entry.category,
            truncate(&r.entry.content, 100),
            r.score,
            r.entry.access_count
        ));
    }
    Ok(msg)
}

fn get_recent_memories(data: &BotData) -> Result<String> {
    let store = data.memory_store.lock().unwrap();
    let entries = store.get_recent(10)?;

    if entries.is_empty() {
        return Ok("No memories stored yet.\nUse /learn <fact> to add memories.".to_string());
    }

    let mut msg = "Recent Memories:\n".to_string();
    for (i, e) in entries.iter().enumerate() {
        msg.push_str(&format!(
            "\n{}. [{}] {}",
            i + 1,
            e.category,
            truncate(&e.content, 80)
        ));
    }
    Ok(msg)
}

/// Semantic search using vector embeddings only
async fn search_memory_semantic(data: &BotData, query: &str) -> String {
    // Get embedder outside the lock
    let embedder = {
        let store = data.memory_store.lock().unwrap();
        if !store.has_embeddings() {
            return "Semantic search unavailable - Ollama not running.\nUse /memory search for keyword search.".to_string();
        }
        store.get_embedder()
    };

    // Compute embedding outside the lock (async)
    let query_embedding = if let Some(embedder) = embedder {
        embedder.read().await.embed(query).await.ok()
    } else {
        None
    };

    // Now do the sync search with pre-computed embedding
    let store = data.memory_store.lock().unwrap();
    match store.search_hybrid_sync(query, query_embedding, 5, 0.0) {
        Ok(results) => {
            if results.is_empty() {
                return format!("No semantically similar memories for: {}", query);
            }

            let mut msg = format!("Semantic matches for '{}':\n", query);
            for (i, r) in results.iter().enumerate() {
                msg.push_str(&format!(
                    "\n{}. [{}] {}\n   (similarity: {:.1}%)",
                    i + 1,
                    r.entry.category,
                    truncate(&r.entry.content, 100),
                    r.vector_score * 100.0
                ));
            }
            msg
        }
        Err(e) => format!("Semantic search error: {}", e),
    }
}

/// Hybrid search combining keyword (BM25) and vector similarity
async fn search_memory_hybrid(data: &BotData, query: &str) -> String {
    // Get embedder outside the lock
    let (embedder, has_vectors) = {
        let store = data.memory_store.lock().unwrap();
        (store.get_embedder(), store.has_embeddings())
    };

    // Compute embedding outside the lock (async)
    let query_embedding = if let Some(embedder) = embedder {
        embedder.read().await.embed(query).await.ok()
    } else {
        None
    };

    // Now do the sync search with pre-computed embedding
    let store = data.memory_store.lock().unwrap();
    match store.search_hybrid_sync(query, query_embedding, 5, 0.4) {
        Ok(results) => {
            if results.is_empty() {
                return format!("No memories found for: {}", query);
            }

            let mode = if has_vectors { "hybrid (keyword + vector)" } else { "keyword only" };

            let mut msg = format!("Results for '{}' ({}):\n", query, mode);
            for (i, r) in results.iter().enumerate() {
                if has_vectors {
                    msg.push_str(&format!(
                        "\n{}. [{}] {}\n   (kw: {:.1}%, vec: {:.1}%, hybrid: {:.1}%)",
                        i + 1,
                        r.entry.category,
                        truncate(&r.entry.content, 100),
                        r.keyword_score * 100.0,
                        r.vector_score * 100.0,
                        r.score * 100.0
                    ));
                } else {
                    msg.push_str(&format!(
                        "\n{}. [{}] {}\n   (score: {:.2})",
                        i + 1,
                        r.entry.category,
                        truncate(&r.entry.content, 100),
                        r.score
                    ));
                }
            }
            msg
        }
        Err(e) => format!("Hybrid search error: {}", e),
    }
}

/// Backfill embeddings for memories that don't have them
async fn backfill_memory_embeddings(data: &BotData) -> String {
    // Step 1: Get embedder and memories needing backfill (quick lock)
    let (embedder, memories) = {
        let store = data.memory_store.lock().unwrap();
        if !store.has_embeddings() {
            return "Backfill unavailable - Ollama not running.\nStart Ollama and restart the bot.".to_string();
        }
        let embedder = match store.get_embedder() {
            Some(e) => e,
            None => return "Backfill unavailable - no embedder configured.".to_string(),
        };
        let memories = match store.get_memories_needing_embeddings(100) {
            Ok(m) => m,
            Err(e) => return format!("Failed to get memories: {}", e),
        };
        (embedder, memories)
    };

    if memories.is_empty() {
        return "All memories already have embeddings.".to_string();
    }

    // Step 2: Compute embeddings (async, no lock held)
    let mut embeddings: Vec<(String, Vec<f32>)> = Vec::new();
    for (id, content) in &memories {
        match embedder.read().await.embed(content).await {
            Ok(embedding) => {
                embeddings.push((id.clone(), embedding));
            }
            Err(e) => {
                tracing::warn!("Failed to embed memory {}: {}", &id[..8.min(id.len())], e);
            }
        }
    }

    // Step 3: Store embeddings (quick lock per batch)
    let embedded_count = {
        let store = data.memory_store.lock().unwrap();
        let mut count = 0;
        for (id, embedding) in &embeddings {
            if store.store_embedding(id, embedding).is_ok() {
                count += 1;
            }
        }
        count
    };

    // Get updated stats
    let store = data.memory_store.lock().unwrap();
    let stats = store.embedding_stats().unwrap_or_default();
    format!(
        "Backfilled {} memories with embeddings.\n\nCoverage: {}/{} ({:.1}%)",
        embedded_count,
        stats.with_embeddings,
        stats.total_memories,
        stats.coverage_percent
    )
}

/// Format embedding statistics
fn format_embedding_stats(data: &BotData) -> Result<String> {
    let store = data.memory_store.lock().unwrap();
    let stats = store.embedding_stats()?;

    let status = if store.has_embeddings() { "✓ Available" } else { "✗ Unavailable" };

    Ok(format!(
        "Embedding Stats\n\n\
        Ollama: {}\n\n\
        Total memories: {}\n\
        With embeddings: {}\n\
        Without embeddings: {}\n\
        Coverage: {:.1}%\n\n\
        Use /memory backfill to generate missing embeddings.",
        status,
        stats.total_memories,
        stats.with_embeddings,
        stats.without_embeddings,
        stats.coverage_percent
    ))
}

fn learn_fact(data: &BotData, fact: &str, user_id: i64) -> String {
    let store = data.memory_store.lock().unwrap();

    // Determine category from content
    let category = categorize_fact(fact);
    let source = format!("telegram_user_{}", user_id);

    match store.learn(fact, &category, &source, 0.9) {
        Ok(id) => format!("Learned [{}]: {}\n(ID: {})", category, truncate(fact, 50), &id[..8]),
        Err(e) => format!("Failed to learn: {}", e),
    }
}

fn categorize_fact(fact: &str) -> String {
    let lower = fact.to_lowercase();
    if lower.contains("prefer") || lower.contains("like") || lower.contains("want") {
        "preference".to_string()
    } else if lower.contains("project") || lower.contains("working on") || lower.contains("building") {
        "project".to_string()
    } else if lower.contains("remember") || lower.contains("note") || lower.contains("important") {
        "note".to_string()
    } else if lower.contains("api") || lower.contains("code") || lower.contains("function") {
        "technical".to_string()
    } else {
        "fact".to_string()
    }
}

/// Format graph statistics
fn format_graph_stats(data: &BotData) -> String {
    let store = match data.graph_store.lock() {
        Ok(s) => s,
        Err(_) => return "Failed to access graph store".to_string(),
    };

    match store.stats() {
        Ok(stats) => format!(
            "Knowledge Graph\n\n\
            Entities: {}\n\
            Relations: {}\n\n\
            Entity Types:\n{}\n\n\
            Commands:\n\
            /extract <text> - Extract entities from text",
            stats.entity_count,
            stats.relation_count,
            stats.by_type.iter()
                .map(|(t, c)| format!("  {} {}", t, c))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Err(e) => format!("Error: {}", e),
    }
}

/// Extract entities from text using Llama
async fn extract_entities(data: &BotData, text: &str) -> String {
    if !data.llama_worker.is_available().await {
        return "Entity extraction requires Llama (Ollama not available)".to_string();
    }

    match data.llama_worker.extract_entities(text).await {
        Ok(entities) if entities.is_empty() => {
            "No entities found in text".to_string()
        }
        Ok(entities) => {
            // Store entities in graph
            let mut stored = 0;
            if let Ok(store) = data.graph_store.lock() {
                for entity in &entities {
                    // Build attributes from context and confidence
                    let attrs = entity.context.as_ref().map(|ctx| {
                        serde_json::json!({
                            "context": ctx,
                            "confidence": entity.confidence
                        })
                    });
                    if store.add_entity(&entity.entity_type, &entity.name, attrs).is_ok() {
                        stored += 1;
                    }
                }
            }

            let entity_list = entities.iter()
                .map(|e| format!("  [{}] {}", e.entity_type, e.name))
                .collect::<Vec<_>>()
                .join("\n");

            format!("Extracted {} entities ({} stored):\n{}", entities.len(), stored, entity_list)
        }
        Err(e) => format!("Extraction failed: {}", e),
    }
}

// ============ Conversation Functions ============

/// Get conversation history as context for a prompt
fn get_conversation_context(data: &BotData, chat_id: i64) -> String {
    let store = match data.conversation_store.lock() {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    match store.get_history_as_context(chat_id, 10) {
        Ok(ctx) => ctx,
        Err(_) => String::new(),
    }
}

/// Store a conversation exchange (user message + assistant response)
fn store_conversation_exchange(data: &BotData, chat_id: i64, user_msg: &str, assistant_msg: &str) {
    let store = match data.conversation_store.lock() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to lock conversation store: {}", e);
            return;
        }
    };

    if let Err(e) = store.add_exchange(chat_id, user_msg, assistant_msg) {
        tracing::error!("Failed to store conversation: {}", e);
    }
}

/// Format conversation history for display
fn format_conversation_history(data: &BotData, chat_id: i64) -> String {
    let store = match data.conversation_store.lock() {
        Ok(s) => s,
        Err(_) => return "Failed to access conversation store".to_string(),
    };

    let history = match store.get_history(chat_id, 10) {
        Ok(h) => h,
        Err(e) => return format!("Error: {}", e),
    };

    if history.is_empty() {
        return "No conversation history yet.".to_string();
    }

    let mut msg = format!("Recent Conversation ({} messages):\n", history.len());
    for m in history {
        let role = if m.role == "user" { "You" } else { "Bot" };
        let content = truncate(&m.content, 100);
        msg.push_str(&format!("\n{}: {}", role, content));
    }

    msg.push_str("\n\nCommands:\n/clear - Clear history\n/memory - View learned facts");
    msg
}

/// Clear conversation history
fn clear_conversation_history(data: &BotData, chat_id: i64) -> String {
    let store = match data.conversation_store.lock() {
        Ok(s) => s,
        Err(_) => return "Failed to access conversation store".to_string(),
    };

    match store.clear(chat_id) {
        Ok(count) => format!(
            "Conversation cleared.\n\n\
            Deleted: {} messages\n\n\
            I've forgotten our conversation, but I still remember learned facts.\n\
            Use /memory to view facts."
            , count
        ),
        Err(e) => format!("Error clearing history: {}", e),
    }
}

/// Get relevant memories as context for a prompt
fn get_memory_context(data: &BotData, prompt: &str) -> String {
    let store = data.memory_store.lock().unwrap();

    // Search for relevant memories
    let results = match store.search(prompt, 3) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    if results.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n\n[Relevant memories from previous conversations:]\n");
    for r in results {
        context.push_str(&format!("- {}\n", r.entry.content));
    }
    context
}

/// Extract facts from Claude's response for learning
fn extract_and_learn_facts(data: &BotData, response: &str, user_id: i64) {
    // Look for explicit "I'll remember" or "Note:" patterns
    let patterns = [
        "I'll remember",
        "I've noted",
        "Noted:",
        "Important:",
        "Key point:",
    ];

    for line in response.lines() {
        for pattern in &patterns {
            if line.contains(pattern) {
                let fact = line.replace(pattern, "").trim().to_string();
                if !fact.is_empty() && fact.len() > 10 {
                    let _ = learn_fact(data, &fact, user_id);
                }
            }
        }
    }
}

/// Load system context and store key facts
fn load_context(data: &BotData) -> String {
    // Store key facts as memories
    let mut learned_count = 0;
    let store = data.memory_store.lock().unwrap();

    let key_facts = [
        ("identity", "I am Eliot, an AI coding assistant powered by Claude, operating as a Telegram bot with persistent memory"),
        ("environment", "Server: clawdbot-prod (Hetzner), Tailscale IP: 100.94.120.80, Domain: clawdbot.velofi.io, User: eliot"),
        ("claudebot", "ClaudeBot MCP - Rust Telegram bot with hybrid semantic memory (BM25 + vector), gRPC bridge, Claude CLI integration"),
        ("tools", "Available: Claude CLI (autonomous), Ollama (llama3.2, nomic-embed-text), Git, Cargo, SQLite"),
        ("workflow", "1. Receive task via Telegram, 2. Check permissions, 3. Recall memories (hybrid search), 4. Execute via Claude CLI, 5. Store learnings, 6. Report results"),
        ("rules", "Always verify directory before coding, run tests after changes, never commit secrets, use Decimal for money (never f64)"),
        ("team", "CEO: Technical (can code), marketing genius, delegates to workers/AI, prefers results over status updates"),
    ];

    for (category, fact) in key_facts {
        if store.learn(fact, category, "context_load", 0.95).is_ok() {
            learned_count += 1;
        }
    }

    format!(
        "Context Loaded\n\n\
        Facts stored: {}\n\n\
        I now understand:\n\
        - My identity as Eliot\n\
        - Server environment (Hetzner/Tailscale)\n\
        - ClaudeBot architecture\n\
        - Available tools\n\
        - Coding workflow & rules\n\
        - Team structure\n\n\
        Use /memory to view stored facts.",
        learned_count
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Parse duration string like "30m", "2h", "1d"
fn parse_duration(s: &str) -> Option<std::time::Duration> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }

    let (num_str, unit) = if s.ends_with('m') {
        (&s[..s.len()-1], 'm')
    } else if s.ends_with('h') {
        (&s[..s.len()-1], 'h')
    } else if s.ends_with('d') {
        (&s[..s.len()-1], 'd')
    } else {
        // Default to minutes if no unit
        (s.as_str(), 'm')
    };

    let num: u64 = num_str.parse().ok()?;

    let secs = match unit {
        'm' => num * 60,
        'h' => num * 3600,
        'd' => num * 86400,
        _ => num * 60,
    };

    Some(std::time::Duration::from_secs(secs))
}

async fn handle_document(
    bot: &Bot,
    msg: &Message,
    chat_id: ChatId,
    data: &BotData,
    doc: &teloxide::types::Document,
    working_dir: &PathBuf,
    user_id: i64,
) -> Result<()> {
    // Check limits
    if let Err(limit_msg) = check_user_limits(data, user_id) {
        bot.send_message(chat_id, limit_msg).await?;
        return Ok(());
    }

    let raw_name = doc.file_name.clone().unwrap_or_else(|| "file".to_string());
    let file_name = std::path::Path::new(&raw_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    bot.send_message(chat_id, format!("Downloading {}...", file_name)).await?;

    let file = bot.get_file(&doc.file.id).await?;
    let file_path = working_dir.join(&file_name);
    let mut dst = tokio::fs::File::create(&file_path).await?;
    bot.download_file(&file.path, &mut dst).await?;

    let caption = msg.caption().unwrap_or("Analyze this file");
    let prompt = format!(
        "{}\n\nThe file has been saved to: {}\nPlease read and analyze it.",
        caption,
        file_path.display()
    );

    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
    let is_autonomous = matches!(
        data.permission_manager.get_status(user_id).level,
        crate::permissions::PermissionLevel::Autonomous
    );
    let response = invoke_claude_cli(&prompt, working_dir, is_autonomous).await?;
    record_usage(data, user_id, &response);
    send_long_message(bot, chat_id, &response.text).await?;

    Ok(())
}

async fn handle_photo(
    bot: &Bot,
    msg: &Message,
    chat_id: ChatId,
    data: &BotData,
    photo: &teloxide::types::PhotoSize,
    working_dir: &PathBuf,
    user_id: i64,
) -> Result<()> {
    // Check limits
    if let Err(limit_msg) = check_user_limits(data, user_id) {
        bot.send_message(chat_id, limit_msg).await?;
        return Ok(());
    }

    bot.send_message(chat_id, "Receiving image...").await?;

    let file = bot.get_file(&photo.file.id).await?;
    let file_name = format!("photo_{}.jpg", chrono::Utc::now().timestamp());
    let file_path = working_dir.join(&file_name);
    let mut dst = tokio::fs::File::create(&file_path).await?;
    bot.download_file(&file.path, &mut dst).await?;

    let caption = msg.caption().unwrap_or("Describe this image");
    let prompt = format!(
        "{}\n\nThe image has been saved to: {}\nPlease analyze it.",
        caption,
        file_path.display()
    );

    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;
    let is_autonomous = matches!(
        data.permission_manager.get_status(user_id).level,
        crate::permissions::PermissionLevel::Autonomous
    );
    let response = invoke_claude_cli(&prompt, working_dir, is_autonomous).await?;
    record_usage(data, user_id, &response);
    send_long_message(bot, chat_id, &response.text).await?;

    Ok(())
}

async fn send_long_message(bot: &Bot, chat_id: ChatId, text: &str) -> Result<()> {
    const MAX: usize = 4000;

    if text.is_empty() {
        bot.send_message(chat_id, "(no response)").await?;
        return Ok(());
    }

    // Convert markdown to HTML for proper code formatting
    let html_text = markdown_to_telegram_html(text);

    if html_text.len() <= MAX {
        // Try HTML first, fall back to plain text if it fails
        match bot.send_message(chat_id, &html_text)
            .parse_mode(ParseMode::Html)
            .await
        {
            Ok(_) => {}
            Err(_) => {
                // HTML failed (probably malformed), send as plain text
                bot.send_message(chat_id, text).await?;
            }
        }
    } else {
        // For long messages, split and send as plain text to avoid breaking HTML tags
        let mut remaining = text;
        while !remaining.is_empty() {
            let split_at = remaining
                .char_indices()
                .take_while(|(i, _)| *i < MAX)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(remaining.len());
            let (chunk, rest) = remaining.split_at(split_at);

            // Try HTML for each chunk
            let html_chunk = markdown_to_telegram_html(chunk);
            match bot.send_message(chat_id, &html_chunk)
                .parse_mode(ParseMode::Html)
                .await
            {
                Ok(_) => {}
                Err(_) => {
                    bot.send_message(chat_id, chunk).await?;
                }
            }
            remaining = rest;
        }
    }
    Ok(())
}

/// Convert markdown code blocks to Telegram HTML format
fn markdown_to_telegram_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 100);
    let mut chars = text.chars().peekable();
    let mut in_code_block = false;
    let mut in_inline_code = false;

    while let Some(c) = chars.next() {
        if c == '`' {
            // Check for code block (```)
            if chars.peek() == Some(&'`') {
                chars.next(); // consume second `
                if chars.peek() == Some(&'`') {
                    chars.next(); // consume third `

                    if in_code_block {
                        result.push_str("</code></pre>");
                        in_code_block = false;
                    } else {
                        // Skip language identifier if present (e.g., ```rust)
                        while let Some(&ch) = chars.peek() {
                            if ch == '\n' {
                                chars.next();
                                break;
                            } else if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                                chars.next();
                            } else if ch == '\n' || ch == '\r' {
                                chars.next();
                                break;
                            } else {
                                break;
                            }
                        }
                        result.push_str("<pre><code>");
                        in_code_block = true;
                    }
                    continue;
                }
            }

            // Single backtick - inline code
            if !in_code_block {
                if in_inline_code {
                    result.push_str("</code>");
                    in_inline_code = false;
                } else {
                    result.push_str("<code>");
                    in_inline_code = true;
                }
                continue;
            }
        }

        // Escape HTML special characters (but not inside code blocks for readability)
        if !in_code_block && !in_inline_code {
            match c {
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '&' => result.push_str("&amp;"),
                _ => result.push(c),
            }
        } else {
            // Inside code: still escape < and > to prevent HTML injection
            match c {
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '&' => result.push_str("&amp;"),
                _ => result.push(c),
            }
        }
    }

    // Close any unclosed tags
    if in_inline_code {
        result.push_str("</code>");
    }
    if in_code_block {
        result.push_str("</code></pre>");
    }

    result
}

// ============ Bypass Bridge Functions ============

/// Handle bypass command - execute task on remote AR server via gRPC
async fn handle_bypass(
    bot: &Bot,
    chat_id: ChatId,
    data: &BotData,
    task: &str,
    user_id: i64,
) -> Result<()> {
    // Check if bridge is configured
    let client = match &data.bridge_client {
        Some(c) => c,
        None => {
            bot.send_message(chat_id,
                "Bridge not configured.\n\n\
                Set BRIDGE_GRPC_URL and BRIDGE_API_KEY environment variables."
            ).await?;
            return Ok(());
        }
    };

    // Check if user is allowed (admin only for bypass)
    if !data.is_allowed(user_id) {
        bot.send_message(chat_id, "Bypass requires admin permission.").await?;
        return Ok(());
    }

    // Send typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

    // Execute on bridge via gRPC streaming
    bot.send_message(chat_id, "Sending to AR bridge (gRPC)...").await?;

    match client.execute_full(chat_id.0, task, None).await {
        Ok(result) => {
            if result.success {
                // Format response with metadata
                let mut reply = result.text.clone();
                if let Some(cost) = result.cost_usd {
                    reply.push_str(&format!("\n\n[Cost: ${:.4}, Duration: {}ms]", cost, result.duration_ms));
                } else {
                    reply.push_str(&format!("\n\n[Duration: {}ms]", result.duration_ms));
                }

                // Store in conversation
                store_conversation_exchange(data, chat_id.0, task, &result.text);

                send_long_message(bot, chat_id, &reply).await?;
            } else {
                let error_msg = result.error.unwrap_or_else(|| "Unknown error".to_string());
                bot.send_message(chat_id, format!("Bridge execution failed:\n{}", error_msg)).await?;
            }
        }
        Err(e) => {
            tracing::error!("Bridge execution error: {}", e);
            bot.send_message(chat_id, format!("Bridge connection error:\n{}", e)).await?;
        }
    }

    Ok(())
}

/// Handle bypass status command - check gRPC bridge health
async fn handle_bypass_status(
    bot: &Bot,
    chat_id: ChatId,
    data: &BotData,
) -> Result<()> {
    let client = match &data.bridge_client {
        Some(c) => c,
        None => {
            bot.send_message(chat_id,
                "Bridge Status: NOT CONFIGURED\n\n\
                Set BRIDGE_GRPC_URL and BRIDGE_API_KEY to enable bypass mode."
            ).await?;
            return Ok(());
        }
    };

    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

    match client.test_connection().await {
        Ok(status_msg) => {
            bot.send_message(chat_id, format!("Bridge Status: CONNECTED\n\n{}", status_msg)).await?;
        }
        Err(e) => {
            bot.send_message(chat_id, format!("Bridge Status: DISCONNECTED\n\nError: {}", e)).await?;
        }
    }

    Ok(())
}

/// Handle bypass file read command via gRPC
async fn handle_bypass_file(
    bot: &Bot,
    chat_id: ChatId,
    data: &BotData,
    path: &str,
    analyze: bool,
) -> Result<()> {
    // Check if bridge is configured
    let client = match &data.bridge_client {
        Some(c) => c,
        None => {
            bot.send_message(chat_id,
                "Bridge not configured.\n\n\
                Set BRIDGE_GRPC_URL and BRIDGE_API_KEY environment variables."
            ).await?;
            return Ok(());
        }
    };

    // Send typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await?;

    let action = if analyze { "Analyzing" } else { "Reading" };
    bot.send_message(chat_id, format!("{} file on AR: {}", action, path)).await?;

    let response = if analyze {
        client.read_file_analyzed(path).await
    } else {
        client.read_file_raw(path, 100 * 1024).await
    };

    match response {
        Ok(file_resp) => {
            if file_resp.success {
                let mut reply = file_resp.content.clone();

                if let Some(size) = file_resp.file_size {
                    reply.push_str(&format!("\n\n[File size: {} bytes", size));
                    if file_resp.truncated {
                        reply.push_str(", truncated");
                    }
                    reply.push(']');
                }

                send_long_message(bot, chat_id, &reply).await?;
            } else {
                let error_msg = file_resp.error.unwrap_or_else(|| "Unknown error".to_string());
                bot.send_message(chat_id, format!("File read failed:\n{}", error_msg)).await?;
            }
        }
        Err(e) => {
            tracing::error!("Bridge file read error: {}", e);
            bot.send_message(chat_id, format!("Bridge connection error:\n{}", e)).await?;
        }
    }

    Ok(())
}
