# ClaudeBot v2 - Complete Implementation Roadmap

> **Style:** Anthropic Internal Prompt Engineering
> **Method:** CIRCLE Development Pipeline für alle Code-Tasks
> **Goal:** Zero Gaps, Production-Ready Telegram Bot

---

## 📋 Executive Summary

**Current State:** Bot startet, empfängt aber keine Telegram-Nachrichten
**Target State:** Voll funktionaler Bot mit allen Industry-Best-Practices

**Timeline:** 4 Epics, 12 Tasks, ~2-3 Tage

---

## 🎯 Epic 1: Telegram Bot Fix (CRITICAL)

> **Priority:** P0 - Blocker
> **Owner:** Claude Code
> **Method:** Debug → Fix → E2E Test

### Task 1.1: Debug Message Polling
```yaml
id: T1.1
type: bug-fix
priority: critical
status: pending
circle: no  # Debug only, no new code

acceptance_criteria:
  - [ ] Identify why teloxide::repl doesn't receive messages
  - [ ] Check bot token validity via Telegram API
  - [ ] Verify no webhook is set (polling requires no webhook)
  - [ ] Check if another bot instance is consuming updates
  - [ ] Add verbose logging to trace message flow

investigation_steps:
  1. curl https://api.telegram.org/bot<TOKEN>/getMe
  2. curl https://api.telegram.org/bot<TOKEN>/getWebhookInfo
  3. curl https://api.telegram.org/bot<TOKEN>/getUpdates
  4. Check teloxide polling configuration
  5. Verify TELEGRAM_ALLOWED_USERS filter
```

### Task 1.2: Fix Teloxide Configuration
```yaml
id: T1.2
type: bug-fix
priority: critical
status: pending
circle: yes  # Code changes require CIRCLE

acceptance_criteria:
  - [ ] Bot receives and logs incoming messages
  - [ ] Bot responds to /start command
  - [ ] Bot processes text messages via Claude CLI
  - [ ] Error handling doesn't silently fail

dependencies:
  - T1.1 (diagnosis complete)
```

### Task 1.3: End-to-End Test Suite
```yaml
id: T1.3
type: test
priority: high
status: pending
circle: yes

acceptance_criteria:
  - [ ] Automated test script verifies full flow
  - [ ] Tests: bot startup, message receive, Claude CLI invoke, response send
  - [ ] Tests run locally before deployment
  - [ ] Clear pass/fail output with diagnostics

deliverables:
  - tests/e2e_telegram.sh (improved)
  - tests/integration/telegram_test.rs
```

---

## 🎯 Epic 2: Missing Core Features

> **Priority:** P1 - Required for Production
> **Method:** CIRCLE Pipeline for each task

### Task 2.1: Wake/Sleep Lifecycle
```yaml
id: T2.1
type: feature
priority: high
status: pending
circle: yes

description: |
  Implement MemGPT-style wake/sleep cycle for background processing:
  - Sleep: Memory consolidation, decay, compression
  - Wake: Active message processing
  - Idle detection with configurable timeout

acceptance_criteria:
  - [ ] src/lifecycle.rs implements state machine (Sleep/Wake/Processing)
  - [ ] Background task consolidates similar memories
  - [ ] Ebbinghaus decay applied during sleep
  - [ ] Old conversations compressed via Llama
  - [ ] Configurable idle timeout (default: 5 min)

files_to_create:
  - src/lifecycle.rs

files_to_modify:
  - src/lib.rs (add module)
  - src/telegram.rs (integrate lifecycle)
```

### Task 2.2: Vector Embeddings
```yaml
id: T2.2
type: feature
priority: medium
status: pending
circle: yes

description: |
  Add semantic search via local embeddings (Ollama nomic-embed-text).
  Hybrid retrieval: FTS5 keyword + vector similarity.

acceptance_criteria:
  - [ ] src/embeddings.rs generates embeddings via Ollama
  - [ ] SQLite stores embedding vectors (BLOB or sqlite-vss)
  - [ ] Cosine similarity search implemented
  - [ ] Hybrid scoring: 0.6 * FTS5 + 0.4 * vector
  - [ ] Falls back to FTS5 if Ollama unavailable

files_to_create:
  - src/embeddings.rs

files_to_modify:
  - src/lib.rs
  - src/memory.rs (integrate embeddings)
```

### Task 2.3: Integrate Token Counter
```yaml
id: T2.3
type: feature
priority: high
status: pending
circle: yes

description: |
  Use TokenCounter for pre-flight budget checks before Claude API calls.
  Warn users when approaching limits.

acceptance_criteria:
  - [ ] Token count estimated before each API call
  - [ ] Warning sent if > 50% of daily budget
  - [ ] Request blocked if would exceed budget
  - [ ] Cost shown in /usage command

files_to_modify:
  - src/telegram.rs (add budget check in handle_text)
  - src/claude.rs (add token counting)
```

### Task 2.4: Integrate Llama Compression
```yaml
id: T2.4
type: feature
priority: high
status: pending
circle: yes

description: |
  Use LlamaWorker for context compression when approaching token limits.
  Auto-compress when context > 4K tokens.

acceptance_criteria:
  - [ ] Context compressed when > 4K tokens
  - [ ] Compression preserves: names, numbers, decisions, TODOs
  - [ ] Falls back to truncation if Ollama unavailable
  - [ ] Entity extraction runs on each response

files_to_modify:
  - src/telegram.rs (integrate compression)
  - src/memory.rs (auto-extract entities)
```

---

## 🎯 Epic 3: Production Hardening

> **Priority:** P1 - Required for Deployment
> **Method:** CIRCLE Pipeline

### Task 3.1: Error Handling & Recovery
```yaml
id: T3.1
type: improvement
priority: high
status: pending
circle: yes

acceptance_criteria:
  - [ ] All errors logged with context
  - [ ] User receives friendly error messages
  - [ ] Bot auto-restarts on crash (systemd)
  - [ ] Rate limiting on Telegram side
  - [ ] Graceful shutdown on SIGTERM

files_to_modify:
  - src/telegram.rs
  - deploy/claudebot-telegram.service
```

### Task 3.2: Metrics & Monitoring
```yaml
id: T3.2
type: feature
priority: medium
status: pending
circle: yes

acceptance_criteria:
  - [ ] /metrics command shows: latency, cache hits, costs
  - [ ] Health check endpoint (for monitoring)
  - [ ] Daily summary sent to user (opt-in)
  - [ ] Alerts on budget threshold

files_to_modify:
  - src/telegram.rs (add /metrics command)
  - src/metrics.rs (telegram integration)
```

### Task 3.3: Security Hardening
```yaml
id: T3.3
type: security
priority: critical
status: pending
circle: yes

acceptance_criteria:
  - [ ] Sensitive data detection before caching
  - [ ] API keys never logged
  - [ ] User isolation (per-user directories)
  - [ ] Rate limiting per user
  - [ ] Input sanitization

files_to_modify:
  - src/telegram.rs
  - src/cache.rs (sensitive data filter)
```

---

## 🎯 Epic 4: Deployment & Validation

> **Priority:** P0 - Final Step
> **Method:** Manual validation

### Task 4.1: Local E2E Test
```yaml
id: T4.1
type: test
priority: critical
status: pending
circle: no

acceptance_criteria:
  - [ ] Bot starts successfully
  - [ ] Receives test message
  - [ ] Invokes Claude CLI
  - [ ] Sends response
  - [ ] Usage tracked correctly
  - [ ] Memory stored correctly

test_script: tests/e2e_telegram.sh
```

### Task 4.2: Deploy to Hetzner
```yaml
id: T4.2
type: deployment
priority: critical
status: pending
circle: no

acceptance_criteria:
  - [ ] Binary deployed via deploy/deploy.sh
  - [ ] Service starts and stays running
  - [ ] Logs show message processing
  - [ ] Response received in Telegram

commands:
  - ./deploy/deploy.sh
  - ssh eliot@100.94.120.80 'journalctl --user -u claudebot-telegram -f'
```

### Task 4.3: Production Validation
```yaml
id: T4.3
type: validation
priority: critical
status: pending
circle: no

acceptance_criteria:
  - [ ] Send "Hallo Eliot" → receive response
  - [ ] /start shows welcome message
  - [ ] /usage shows token stats
  - [ ] /memory shows memory stats
  - [ ] File upload works
  - [ ] Image analysis works
  - [ ] Budget limits respected
```

---

## 📊 Task Dependency Graph

```
Epic 1: Telegram Fix
┌─────────┐     ┌─────────┐     ┌─────────┐
│  T1.1   │────►│  T1.2   │────►│  T1.3   │
│ Debug   │     │  Fix    │     │  E2E    │
└─────────┘     └─────────┘     └─────────┘
                     │
                     ▼
Epic 2: Core Features
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  T2.1   │     │  T2.2   │     │  T2.3   │     │  T2.4   │
│ Wake/   │     │ Vector  │     │ Token   │     │ Llama   │
│ Sleep   │     │ Embed   │     │ Counter │     │ Compress│
└─────────┘     └─────────┘     └─────────┘     └─────────┘
     │               │               │               │
     └───────────────┴───────────────┴───────────────┘
                           │
                           ▼
Epic 3: Production Hardening
┌─────────┐     ┌─────────┐     ┌─────────┐
│  T3.1   │     │  T3.2   │     │  T3.3   │
│ Errors  │     │ Metrics │     │Security │
└─────────┘     └─────────┘     └─────────┘
     │               │               │
     └───────────────┴───────────────┘
                     │
                     ▼
Epic 4: Deployment
┌─────────┐     ┌─────────┐     ┌─────────┐
│  T4.1   │────►│  T4.2   │────►│  T4.3   │
│Local E2E│     │ Deploy  │     │ Validate│
└─────────┘     └─────────┘     └─────────┘
```

---

## 🔄 CIRCLE Pipeline Usage

> **WICHTIG:** Alle Code-Tasks (T1.2, T2.x, T3.x) durchlaufen den CIRCLE:

```
/circle <task-description>

Pipeline:
1. [1/5] Graydon - Implementation
2. [2/5] Linus - Code Review
3. [3/5] Maria - Testing
4. [4/5] Kai - Optimization
5. [5/5] Sentinel - Security
```

**Tasks ohne CIRCLE:**
- T1.1 (Debug only)
- T4.1, T4.2, T4.3 (Deployment/Validation)

---

## 📅 Execution Order

### Day 1: Epic 1 (Telegram Fix)
1. T1.1: Debug message polling (30 min)
2. T1.2: Fix teloxide config (`/circle`) (1-2 hrs)
3. T1.3: E2E test suite (`/circle`) (1 hr)

### Day 2: Epic 2 (Core Features)
4. T2.1: Wake/Sleep lifecycle (`/circle`) (2 hrs)
5. T2.2: Vector embeddings (`/circle`) (2 hrs)
6. T2.3: Token counter integration (`/circle`) (1 hr)
7. T2.4: Llama compression (`/circle`) (1 hr)

### Day 3: Epic 3 + 4 (Hardening & Deploy)
8. T3.1: Error handling (`/circle`) (1 hr)
9. T3.2: Metrics (`/circle`) (1 hr)
10. T3.3: Security (`/circle`) (1 hr)
11. T4.1: Local E2E test (30 min)
12. T4.2: Deploy to Hetzner (30 min)
13. T4.3: Production validation (30 min)

---

## ✅ Success Criteria

**Bot ist production-ready wenn:**

1. ✅ Telegram messages werden empfangen und verarbeitet
2. ✅ Claude CLI wird korrekt aufgerufen
3. ✅ Token/Budget tracking funktioniert
4. ✅ Memory system speichert und retrieved
5. ✅ Graph memory extrahiert Entities
6. ✅ Wake/Sleep cycle läuft im Hintergrund
7. ✅ Vector search verbessert retrieval
8. ✅ Alle errors werden geloggt und behandelt
9. ✅ Security checks bestanden
10. ✅ E2E tests pass lokal und remote

---

## 📝 Notes

- **CIRCLE Skill:** Verwende `/circle <feature>` für alle Implementation tasks
- **Parallel Work:** T2.1-T2.4 können parallel entwickelt werden
- **Blocker:** T1.x muss zuerst abgeschlossen werden
- **Rollback:** Bei Problemen: `systemctl --user stop claudebot-telegram`

---

*Created: 2026-01-28*
*Last Updated: 2026-01-28*
*Status: Ready for Execution*
