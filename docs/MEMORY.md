# Conversation Memory Documentation

> SQLite-based conversation persistence for Telegram chat continuity.

---

## Overview

The conversation store solves the "memory loss" problem where Claude would forget context between messages:

```
BEFORE:
  User: "My name is Max"
  Bot:  "Nice to meet you, Max!"
  User: "What's my name?"
  Bot:  "I don't know your name." ← PROBLEM

AFTER:
  User: "My name is Max"
  Bot:  "Nice to meet you, Max!"
  User: "What's my name?"
  Bot:  "Your name is Max!" ← FIXED
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  MEMORY FLOW                                                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  User Message                                                       │
│       │                                                             │
│       ▼                                                             │
│  ┌─────────────────────────────────────┐                           │
│  │  Load History (last 50 messages)    │◄──── SQLite DB            │
│  └─────────────────────────────────────┘                           │
│       │                                                             │
│       ▼                                                             │
│  ┌─────────────────────────────────────┐                           │
│  │  Build Prompt with Context          │                           │
│  │                                     │                           │
│  │  [Previous conversation:]           │                           │
│  │  User: Hello, I'm Max               │                           │
│  │  Assistant: Nice to meet you!       │                           │
│  │                                     │                           │
│  │  [Current message:]                 │                           │
│  │  User: What's my name?              │                           │
│  └─────────────────────────────────────┘                           │
│       │                                                             │
│       ▼                                                             │
│  ┌─────────────────────────────────────┐                           │
│  │  Claude CLI                         │                           │
│  └─────────────────────────────────────┘                           │
│       │                                                             │
│       ▼                                                             │
│  ┌─────────────────────────────────────┐                           │
│  │  Save Exchange to History           │───► SQLite DB            │
│  └─────────────────────────────────────┘                           │
│       │                                                             │
│       ▼                                                             │
│  Response to User                                                   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Database Schema

```sql
CREATE TABLE conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    timestamp INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_conversations_chat_id ON conversations(chat_id);
CREATE INDEX idx_conversations_timestamp ON conversations(chat_id, timestamp DESC);
```

---

## API Reference

### ConversationStore

```rust
pub struct ConversationStore {
    conn: Connection,
    max_messages: usize,  // Default: 50
    ttl_seconds: i64,     // Default: 7 days
}
```

### Methods

| Method | Description |
|--------|-------------|
| `open(path)` | Open or create database |
| `open_with_config(path, max, ttl)` | Open with custom limits |
| `add_message(chat_id, role, content)` | Add single message |
| `add_exchange(chat_id, user, assistant)` | Add user+assistant pair atomically |
| `get_history(chat_id, limit)` | Get recent messages |
| `get_history_as_context(chat_id, limit)` | Get formatted for prompt injection |
| `clear(chat_id)` | Clear chat history |
| `get_summary(chat_id)` | Get message count and timestamps |
| `cleanup_expired()` | Remove messages older than TTL |
| `stats()` | Global statistics |

---

## Telegram Commands

| Command | Description |
|---------|-------------|
| `/memory` | Show memory stats for current chat |
| `/history` | Show last 10 messages |
| `/clear` | Clear conversation history (requires confirmation) |
| `/context` | Load context from file on startup |

### Examples

```
/memory
📊 Memory Stats
Messages: 47
Oldest: 2 hours ago
Newest: just now
TTL: 6 days remaining

/history
📜 Recent History
1. User: Hello, I'm Max
2. Assistant: Nice to meet you, Max!
3. User: What should I work on?
4. ...

/clear
⚠️ Are you sure? This will delete 47 messages.
Reply with "yes" to confirm.
```

---

## Configuration

```bash
# .env

# Database path
CONVERSATION_DB_PATH=/home/eliot/workspace/conversations.db

# Rolling window size (messages per chat)
CONVERSATION_MAX_MESSAGES=50

# Time-to-live in seconds (default: 7 days)
CONVERSATION_TTL=604800
```

---

## Features

### Rolling Window

Messages are automatically trimmed to keep only the most recent N messages per chat:

- Default: 50 messages
- Oldest messages deleted when limit exceeded
- Prevents unbounded database growth

### Multi-Chat Isolation

Each chat has independent history:

- Chat 111: Has its own 50 messages
- Chat 222: Has its own 50 messages
- Clearing one doesn't affect others

### TTL Cleanup

Old conversations are automatically purged:

- Default: 7 days
- Background cleanup on startup
- Configurable via `CONVERSATION_TTL`

### Message Truncation

Long messages are truncated in context to prevent token explosion:

- Messages over 500 chars truncated with "..."
- Full content preserved in database
- Only truncated in prompt injection

### Atomic Exchanges

User + Assistant pairs stored atomically:

```rust
store.add_exchange(chat_id, "Hello!", "Hi there!")?;
// Both messages saved or neither (transaction)
```

---

## Context Format

History is injected into prompts in this format:

```
[Previous conversation:]
User: Hello, I'm Max
Assistant: Nice to meet you, Max!
User: What should I work on today?
Assistant: Let me check your tasks...

[Current message:]
User: What's my name?
```

---

## Troubleshooting

### "Memory not persisting"

1. Check database path exists and is writable
2. Verify `CONVERSATION_DB_PATH` is set
3. Check file permissions

### "Memory seems limited"

- Default is 50 messages (rolling window)
- Increase with `CONVERSATION_MAX_MESSAGES`

### "Old conversations disappeared"

- TTL cleanup removes messages older than 7 days
- Adjust with `CONVERSATION_TTL`

### Database corruption

```bash
# Backup and recreate
cp conversations.db conversations.db.bak
rm conversations.db
# Restart bot - will create fresh database
```

---

## Performance

| Operation | Typical Time |
|-----------|--------------|
| Add message | < 1ms |
| Get history (50 msgs) | < 5ms |
| Clear chat | < 1ms |
| Cleanup expired | < 10ms |

SQLite indexes ensure fast lookups by chat_id and timestamp.

---

*Last Updated: 2026-01-29*
