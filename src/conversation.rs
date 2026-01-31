//! Conversation Store
//!
//! Stores actual message turns per chat for conversation continuity.
//! Unlike MemoryStore (semantic facts), this stores raw message history.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info};

/// Maximum messages to keep per conversation (rolling window)
const MAX_MESSAGES_PER_CONVERSATION: usize = 50;

/// Default TTL in seconds (7 days)
const DEFAULT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

/// A single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,      // "user" or "assistant"
    pub content: String,
    pub timestamp: i64,    // Unix timestamp
}

/// Summary of a conversation
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub chat_id: i64,
    pub message_count: usize,
    pub oldest_timestamp: Option<i64>,
    pub newest_timestamp: Option<i64>,
}

/// Conversation store with SQLite backend
pub struct ConversationStore {
    conn: Connection,
    max_messages: usize,
    ttl_seconds: i64,
}

impl ConversationStore {
    /// Open or create conversation database
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let store = Self {
            conn,
            max_messages: MAX_MESSAGES_PER_CONVERSATION,
            ttl_seconds: DEFAULT_TTL_SECONDS,
        };
        store.init_schema()?;

        info!("Conversation store opened: {}", path.display());
        Ok(store)
    }

    /// Open with custom limits
    pub fn open_with_config(path: &Path, max_messages: usize, ttl_seconds: i64) -> Result<Self> {
        let mut store = Self::open(path)?;
        store.max_messages = max_messages;
        store.ttl_seconds = ttl_seconds;
        Ok(store)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_id INTEGER NOT NULL,
                role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_conversations_chat_id
                ON conversations(chat_id);
            CREATE INDEX IF NOT EXISTS idx_conversations_timestamp
                ON conversations(chat_id, timestamp DESC);
            "#,
        )?;

        Ok(())
    }

    /// Add a message to a conversation
    pub fn add_message(&self, chat_id: i64, role: &str, content: &str) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp_millis(); // Use milliseconds for uniqueness

        self.conn.execute(
            "INSERT INTO conversations (chat_id, role, content, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![chat_id, role, content, timestamp],
        )?;

        // Trim old messages if over limit
        self.trim_default(chat_id)?;

        debug!("Added {} message to chat {}", role, chat_id);
        Ok(())
    }

    /// Add a complete exchange (user message + assistant response) atomically
    pub fn add_exchange(&self, chat_id: i64, user_msg: &str, assistant_msg: &str) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp_millis(); // Use milliseconds

        // Use transaction for atomicity
        self.conn.execute("BEGIN", [])?;

        let result = (|| -> Result<()> {
            self.conn.execute(
                "INSERT INTO conversations (chat_id, role, content, timestamp)
                 VALUES (?1, 'user', ?2, ?3)",
                params![chat_id, user_msg, timestamp],
            )?;

            self.conn.execute(
                "INSERT INTO conversations (chat_id, role, content, timestamp)
                 VALUES (?1, 'assistant', ?2, ?3)",
                params![chat_id, assistant_msg, timestamp + 1], // +1ms to ensure ordering
            )?;

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                self.trim_default(chat_id)?;
                debug!("Added exchange to chat {}", chat_id);
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }

    /// Get conversation history for a chat
    pub fn get_history(&self, chat_id: i64, limit: usize) -> Result<Vec<ConversationMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, timestamp FROM conversations
             WHERE chat_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;

        let messages: Vec<ConversationMessage> = stmt
            .query_map(params![chat_id, limit], |row| {
                Ok(ConversationMessage {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    timestamp: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Reverse to get chronological order
        let mut messages = messages;
        messages.reverse();

        Ok(messages)
    }

    /// Get conversation history formatted for Claude prompt injection
    pub fn get_history_as_context(&self, chat_id: i64, limit: usize) -> Result<String> {
        let messages = self.get_history(chat_id, limit)?;

        if messages.is_empty() {
            return Ok(String::new());
        }

        let mut context = String::from("\n\n[Previous conversation:]\n");
        for msg in messages {
            let role_label = if msg.role == "user" { "User" } else { "Assistant" };
            // Truncate very long messages in history (UTF-8 safe)
            let content = if msg.content.len() > 500 {
                // Find a safe UTF-8 boundary near 500 bytes
                let truncate_at = msg.content
                    .char_indices()
                    .take_while(|(i, _)| *i < 500)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(500.min(msg.content.len()));
                format!("{}...", &msg.content[..truncate_at])
            } else {
                msg.content.clone()
            };
            context.push_str(&format!("{}: {}\n", role_label, content));
        }
        context.push_str("\n[Current message:]\n");

        Ok(context)
    }

    /// Clear conversation history for a chat
    pub fn clear(&self, chat_id: i64) -> Result<usize> {
        let rows = self.conn.execute(
            "DELETE FROM conversations WHERE chat_id = ?1",
            params![chat_id],
        )?;
        info!("Cleared {} messages from chat {}", rows, chat_id);
        Ok(rows)
    }

    /// Get conversation summary
    pub fn get_summary(&self, chat_id: i64) -> Result<ConversationSummary> {
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*), MIN(timestamp), MAX(timestamp)
             FROM conversations WHERE chat_id = ?1",
        )?;

        let summary = stmt.query_row(params![chat_id], |row| {
            Ok(ConversationSummary {
                chat_id,
                message_count: row.get::<_, i64>(0)? as usize,
                oldest_timestamp: row.get(1)?,
                newest_timestamp: row.get(2)?,
            })
        })?;

        Ok(summary)
    }

    /// Trim conversation to a specific number of messages
    pub fn trim_conversation(&self, chat_id: i64, keep_count: usize) -> Result<usize> {
        let rows = self.conn.execute(
            "DELETE FROM conversations
             WHERE chat_id = ?1 AND id NOT IN (
                 SELECT id FROM conversations
                 WHERE chat_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2
             )",
            params![chat_id, keep_count],
        )?;
        Ok(rows)
    }

    /// Get chat IDs with old conversations that have many messages
    /// Returns chats older than `age_seconds` with more than `min_messages`
    pub fn get_stale_conversations(&self, age_seconds: i64, min_messages: usize) -> Result<Vec<i64>> {
        let cutoff = chrono::Utc::now().timestamp_millis() - (age_seconds * 1000);

        let mut stmt = self.conn.prepare(
            "SELECT chat_id, COUNT(*) as msg_count, MAX(timestamp) as last_msg
             FROM conversations
             GROUP BY chat_id
             HAVING msg_count > ?1 AND last_msg < ?2
             ORDER BY msg_count DESC"
        )?;

        let chats = stmt.query_map(params![min_messages as i64, cutoff], |row| {
            row.get::<_, i64>(0)
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(chats)
    }

    /// Internal trim (uses default max)
    fn trim_default(&self, chat_id: i64) -> Result<()> {
        self.trim_conversation(chat_id, self.max_messages)?;
        Ok(())
    }

    /// Clean up expired conversations (older than TTL)
    pub fn cleanup_expired(&self) -> Result<usize> {
        // Convert TTL to milliseconds since we store timestamp_millis
        let cutoff = chrono::Utc::now().timestamp_millis() - (self.ttl_seconds * 1000);
        let rows = self.conn.execute(
            "DELETE FROM conversations WHERE timestamp < ?1",
            params![cutoff],
        )?;
        if rows > 0 {
            info!("Cleaned up {} expired conversation messages", rows);
        }
        Ok(rows)
    }

    /// Get total stats
    pub fn stats(&self) -> Result<ConversationStats> {
        let total_messages: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))?;

        let total_chats: i64 = self
            .conn
            .query_row("SELECT COUNT(DISTINCT chat_id) FROM conversations", [], |row| row.get(0))?;

        Ok(ConversationStats {
            total_messages: total_messages as usize,
            total_chats: total_chats as usize,
        })
    }
}

/// Global conversation statistics
#[derive(Debug, Clone)]
pub struct ConversationStats {
    pub total_messages: usize,
    pub total_chats: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db(name: &str) -> ConversationStore {
        let path = PathBuf::from(format!("/tmp/claudebot_conv_test_{}.db", name));
        let _ = std::fs::remove_file(&path);
        ConversationStore::open(&path).unwrap()
    }

    #[test]
    fn test_add_and_get_history() {
        let store = temp_db("history");
        let chat_id = 12345;

        store.add_message(chat_id, "user", "Hello, my name is Max").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.add_message(chat_id, "assistant", "Nice to meet you, Max!").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.add_message(chat_id, "user", "What's my name?").unwrap();

        let history = store.get_history(chat_id, 10).unwrap();
        assert_eq!(history.len(), 3);
        // First message should be user with "Max"
        assert_eq!(history[0].role, "user");
        assert!(history[0].content.contains("Max"), "First message should contain 'Max', got: {}", history[0].content);
        // Last message should be user asking about name
        assert_eq!(history[2].role, "user");
        assert!(history[2].content.contains("name"), "Last message should contain 'name', got: {}", history[2].content);
    }

    #[test]
    fn test_add_exchange() {
        let store = temp_db("exchange");
        let chat_id = 12345;

        store.add_exchange(chat_id, "Hello!", "Hi there!").unwrap();
        store.add_exchange(chat_id, "How are you?", "I'm doing great!").unwrap();

        let history = store.get_history(chat_id, 10).unwrap();
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn test_history_as_context() {
        let store = temp_db("context");
        let chat_id = 12345;

        store.add_message(chat_id, "user", "My name is Max").unwrap();
        store.add_message(chat_id, "assistant", "Hello Max!").unwrap();

        let context = store.get_history_as_context(chat_id, 10).unwrap();
        assert!(context.contains("[Previous conversation:]"));
        assert!(context.contains("User: My name is Max"));
        assert!(context.contains("Assistant: Hello Max!"));
        assert!(context.contains("[Current message:]"));
    }

    #[test]
    fn test_clear() {
        let store = temp_db("clear");
        let chat_id = 12345;

        store.add_message(chat_id, "user", "Test 1").unwrap();
        store.add_message(chat_id, "user", "Test 2").unwrap();

        let cleared = store.clear(chat_id).unwrap();
        assert_eq!(cleared, 2);

        let history = store.get_history(chat_id, 10).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn test_trim_conversation() {
        let path = PathBuf::from("/tmp/claudebot_conv_test_trim.db");
        let _ = std::fs::remove_file(&path);
        let store = ConversationStore::open_with_config(&path, 3, DEFAULT_TTL_SECONDS).unwrap();
        let chat_id = 12345;

        // Add more than max
        for i in 0..5 {
            store.add_message(chat_id, "user", &format!("Message {}", i)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure different timestamps
        }

        let history = store.get_history(chat_id, 10).unwrap();
        assert_eq!(history.len(), 3); // Only last 3 kept
        assert!(history[2].content.contains("Message 4")); // Most recent
    }

    #[test]
    fn test_multi_chat_isolation() {
        let store = temp_db("isolation");

        store.add_message(111, "user", "Chat 1 message").unwrap();
        store.add_message(222, "user", "Chat 2 message").unwrap();

        let history1 = store.get_history(111, 10).unwrap();
        let history2 = store.get_history(222, 10).unwrap();

        assert_eq!(history1.len(), 1);
        assert_eq!(history2.len(), 1);
        assert!(history1[0].content.contains("Chat 1"));
        assert!(history2[0].content.contains("Chat 2"));
    }

    #[test]
    fn test_summary() {
        let store = temp_db("summary");
        let chat_id = 12345;

        store.add_exchange(chat_id, "Hello", "Hi").unwrap();

        let summary = store.get_summary(chat_id).unwrap();
        assert_eq!(summary.message_count, 2);
        assert!(summary.oldest_timestamp.is_some());
        assert!(summary.newest_timestamp.is_some());
    }
}
