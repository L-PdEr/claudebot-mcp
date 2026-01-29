//! Conversation Store Integration Tests
//!
//! Tests for conversation persistence and retrieval.

use claudebot_mcp::conversation::{ConversationStore, ConversationMessage};
use std::path::PathBuf;
use tempfile::TempDir;

fn create_test_store(name: &str) -> (ConversationStore, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join(format!("{}.db", name));
    let store = ConversationStore::open(&db_path).expect("Failed to create store");
    (store, temp_dir)
}

#[test]
fn test_store_and_retrieve_messages() {
    let (store, _temp) = create_test_store("retrieve");
    let chat_id = 12345;

    // Add messages
    store.add_message(chat_id, "user", "Hello, my name is Max").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.add_message(chat_id, "assistant", "Nice to meet you, Max!").unwrap();

    // Retrieve
    let history = store.get_history(chat_id, 10).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert!(history[0].content.contains("Max"));
    assert_eq!(history[1].role, "assistant");
}

#[test]
fn test_add_exchange() {
    let (store, _temp) = create_test_store("exchange");
    let chat_id = 12345;

    store.add_exchange(chat_id, "What's 2+2?", "2+2 equals 4.").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5)); // Ensure timestamp separation
    store.add_exchange(chat_id, "And 3+3?", "3+3 equals 6.").unwrap();

    let history = store.get_history(chat_id, 10).unwrap();
    assert_eq!(history.len(), 4);

    // Check order (should be chronological)
    assert_eq!(history[0].role, "user");
    assert!(history[0].content.contains("2+2"));
    assert_eq!(history[1].role, "assistant");
    assert!(history[1].content.contains("4"));
}

#[test]
fn test_rolling_window() {
    let (store, _temp) = create_test_store("rolling");
    let chat_id = 12345;

    // Add more messages than the default limit (50)
    // Using open_with_config with max_messages=5 for faster testing
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("rolling.db");
    let store = ConversationStore::open_with_config(&db_path, 5, 86400).unwrap();

    for i in 0..10 {
        store.add_message(chat_id, "user", &format!("Message {}", i)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let history = store.get_history(chat_id, 100).unwrap();
    assert_eq!(history.len(), 5); // Only last 5 kept

    // Should have messages 5-9 (the most recent)
    assert!(history[4].content.contains("Message 9"));
}

#[test]
fn test_clear_conversation() {
    let (store, _temp) = create_test_store("clear");
    let chat_id = 12345;

    store.add_message(chat_id, "user", "Test 1").unwrap();
    store.add_message(chat_id, "user", "Test 2").unwrap();
    store.add_message(chat_id, "user", "Test 3").unwrap();

    let cleared = store.clear(chat_id).unwrap();
    assert_eq!(cleared, 3);

    let history = store.get_history(chat_id, 10).unwrap();
    assert!(history.is_empty());
}

#[test]
fn test_multi_chat_isolation() {
    let (store, _temp) = create_test_store("isolation");

    let chat_1 = 111;
    let chat_2 = 222;
    let chat_3 = 333;

    store.add_message(chat_1, "user", "Chat 1 message").unwrap();
    store.add_message(chat_2, "user", "Chat 2 message").unwrap();
    store.add_message(chat_3, "user", "Chat 3 message").unwrap();

    let history_1 = store.get_history(chat_1, 10).unwrap();
    let history_2 = store.get_history(chat_2, 10).unwrap();
    let history_3 = store.get_history(chat_3, 10).unwrap();

    assert_eq!(history_1.len(), 1);
    assert_eq!(history_2.len(), 1);
    assert_eq!(history_3.len(), 1);

    assert!(history_1[0].content.contains("Chat 1"));
    assert!(history_2[0].content.contains("Chat 2"));
    assert!(history_3[0].content.contains("Chat 3"));

    // Clear one chat doesn't affect others
    store.clear(chat_2).unwrap();

    assert_eq!(store.get_history(chat_1, 10).unwrap().len(), 1);
    assert_eq!(store.get_history(chat_2, 10).unwrap().len(), 0);
    assert_eq!(store.get_history(chat_3, 10).unwrap().len(), 1);
}

#[test]
fn test_get_summary() {
    let (store, _temp) = create_test_store("summary");
    let chat_id = 12345;

    store.add_exchange(chat_id, "Hello", "Hi there!").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.add_exchange(chat_id, "How are you?", "I'm good!").unwrap();

    let summary = store.get_summary(chat_id).unwrap();
    assert_eq!(summary.chat_id, chat_id);
    assert_eq!(summary.message_count, 4);
    assert!(summary.oldest_timestamp.is_some());
    assert!(summary.newest_timestamp.is_some());
    assert!(summary.newest_timestamp.unwrap() >= summary.oldest_timestamp.unwrap());
}

#[test]
fn test_get_history_as_context() {
    let (store, _temp) = create_test_store("context");
    let chat_id = 12345;

    store.add_message(chat_id, "user", "My name is Alice").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.add_message(chat_id, "assistant", "Hello Alice!").unwrap();

    let context = store.get_history_as_context(chat_id, 10).unwrap();

    assert!(context.contains("[Previous conversation:]"));
    assert!(context.contains("User: My name is Alice"));
    assert!(context.contains("Assistant: Hello Alice!"));
    assert!(context.contains("[Current message:]"));
}

#[test]
fn test_empty_history_returns_empty_context() {
    let (store, _temp) = create_test_store("empty_context");
    let chat_id = 99999;

    let context = store.get_history_as_context(chat_id, 10).unwrap();
    assert!(context.is_empty());
}

#[test]
fn test_stats() {
    let (store, _temp) = create_test_store("stats");

    store.add_message(111, "user", "Chat 1").unwrap();
    store.add_message(222, "user", "Chat 2").unwrap();
    store.add_message(222, "assistant", "Response 2").unwrap();
    store.add_message(333, "user", "Chat 3").unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.total_messages, 4);
    assert_eq!(stats.total_chats, 3);
}

#[test]
fn test_long_message_truncation_in_context() {
    let (store, _temp) = create_test_store("truncation");
    let chat_id = 12345;

    // Create a very long message (> 500 chars)
    let long_message = "A".repeat(1000);
    store.add_message(chat_id, "user", &long_message).unwrap();

    let context = store.get_history_as_context(chat_id, 10).unwrap();

    // The context should truncate long messages
    assert!(context.contains("User: "));
    assert!(context.contains("..."));
    // Should not contain the full 1000 chars
    assert!(context.len() < 1000);
}
