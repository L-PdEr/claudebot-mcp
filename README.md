# ClaudeBot MCP Server

All-Rust Model Context Protocol (MCP) server for Claude Code integration.

## Features

- **MCP Protocol**: JSON-RPC 2.0 over stdio
- **Prompt Caching**: 90% cost reduction via `cache_control: ephemeral`
- **Model Router**: Keyword + optional Llama-based complexity classification
- **Memory Store**: SQLite + FTS5 full-text search
- **Graph Memory**: Entity extraction and relationship tracking
- **Response Cache**: SHA256 context-aware caching with Moka LRU
- **Development Circle**: 5-persona quality pipeline
- **Metrics**: Cost tracking and performance monitoring
- **Telegram Bot**: Direct chat interface with conversation memory
- **Bypass Bridge**: Remote Claude Code execution between servers

---

## Quick Start

### 1. Prerequisites

```bash
# Rust toolchain (1.75+)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify installation
rustc --version  # Should be 1.75.0 or higher
```

### 2. Build

```bash
cd claudebot-mcp

# Development build
cargo build

# Production build (optimized, ~3MB binary)
cargo build --release
```

The binary will be at:
- Debug: `target/debug/claudebot-mcp`
- Release: `target/release/claudebot-mcp`

### 3. Configure

```bash
# Copy example config
cp .env.example .env

# Edit with your API key
nano .env
```

**Required configuration:**
```bash
# .env file
ANTHROPIC_API_KEY=sk-ant-api03-xxxxx
```

**Optional configuration:**
```bash
# Default model: haiku, sonnet, or opus (default: opus)
CLAUDEBOT_MODEL=opus

# Ollama URL for local Llama classification (optional)
CLAUDEBOT_OLLAMA_URL=http://localhost:11434

# Database path (default: ./data/claudebot.db)
CLAUDEBOT_DB_PATH=./data/claudebot.db

# Response cache settings
CLAUDEBOT_CACHE_ENABLED=true
CLAUDEBOT_CACHE_TTL=3600
```

### 4. Test the Server

```bash
# Run directly (will wait for MCP input on stdin)
./target/release/claudebot-mcp

# Test with echo (should return MCP response)
echo '{"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{}},"id":1}' | ./target/release/claudebot-mcp
```

### 5. Configure Claude Code

Add to your Claude Code MCP configuration:

**Location:** `~/.config/claude-code/config.json` (Linux/macOS)

```json
{
  "mcpServers": {
    "claudebot": {
      "command": "/absolute/path/to/claudebot-mcp/target/release/claudebot-mcp",
      "env": {
        "ANTHROPIC_API_KEY": "sk-ant-api03-xxxxx"
      }
    }
  }
}
```

**Alternative with environment variable:**
```json
{
  "mcpServers": {
    "claudebot": {
      "command": "/absolute/path/to/claudebot-mcp/target/release/claudebot-mcp",
      "env": {
        "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"
      }
    }
  }
}
```

### 6. Verify in Claude Code

```bash
# Restart Claude Code to load the new MCP server
claude

# Check available tools (should show claudebot tools)
/mcp
```

---

## Installation Options

### Option A: Local Development

```bash
# Clone and build
git clone <repo>
cd quantum-nexus-trading/claudebot-mcp
cargo build --release

# Configure
cp .env.example .env
# Edit .env with your ANTHROPIC_API_KEY
```

### Option B: System-wide Installation

```bash
# Build release
cargo build --release

# Install to /usr/local/bin
sudo cp target/release/claudebot-mcp /usr/local/bin/

# Create config directory
mkdir -p ~/.config/claudebot-mcp
cp .env.example ~/.config/claudebot-mcp/.env

# Update Claude Code config to use system binary
# command: "/usr/local/bin/claudebot-mcp"
```

### Option C: With Ollama (Local LLM Router)

For enhanced model routing with local Llama classification:

```bash
# Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# Pull Llama model for classification
ollama pull llama3.2:3b

# Start Ollama server
ollama serve

# Configure claudebot-mcp
echo 'CLAUDEBOT_OLLAMA_URL=http://localhost:11434' >> .env
```

---

## Directory Structure

```
claudebot-mcp/
├── Cargo.toml          # Rust dependencies
├── .env.example        # Configuration template
├── .env                # Your configuration (create this)
├── src/
│   ├── main.rs         # Entry point
│   ├── lib.rs          # Library exports
│   ├── config.rs       # Configuration loading
│   ├── mcp.rs          # MCP protocol handler
│   ├── tools.rs        # Tool registry (20+ tools)
│   ├── memory.rs       # SQLite + FTS5 memory
│   ├── graph.rs        # Knowledge graph (E3)
│   ├── cache.rs        # Response cache (E4)
│   ├── router.rs       # Model router (E2)
│   ├── claude.rs       # Claude API client (E1)
│   ├── circle.rs       # Development Circle (E5)
│   └── metrics.rs      # Cost/latency tracking (E6)
├── data/               # SQLite databases (auto-created)
└── target/             # Build output
```

---

## MCP Tools Reference

### Router
| Tool | Description |
|------|-------------|
| `router_classify` | Route message to target with model hint |

### Memory
| Tool | Description |
|------|-------------|
| `memory_learn` | Store fact with category and confidence |
| `memory_search` | FTS5 full-text search |
| `memory_recall` | Get recent memories by category |
| `memory_forget` | Delete memory by ID |
| `memory_stats` | Get memory statistics |

### Graph
| Tool | Description |
|------|-------------|
| `graph_add_entity` | Add entity to knowledge graph |
| `graph_add_relation` | Create relationship between entities |
| `graph_find_entity` | Find entity by name |
| `graph_traverse` | Traverse graph (1-2 hops) |
| `graph_entities_by_type` | List entities by type |
| `graph_extract` | Extract entities from text |
| `graph_stats` | Get graph statistics |

### Cache
| Tool | Description |
|------|-------------|
| `cache_stats` | Get response cache stats |
| `cache_clear` | Clear response cache |

### Circle
| Tool | Description |
|------|-------------|
| `circle_run` | Run 5-phase quality pipeline |

### Metrics
| Tool | Description |
|------|-------------|
| `metrics_quick` | Quick stats summary |
| `metrics_cost` | Cost breakdown (day/week/month) |
| `metrics_latency` | Latency percentiles (p50/p90/p99) |
| `metrics_export` | Export all metrics as JSON |
| `metrics_reset` | Reset all metrics |

### Claude
| Tool | Description |
|------|-------------|
| `claude_complete` | Send prompt with caching |

---

## Model Selection

| Model | Use Case | Input Cost | Output Cost |
|-------|----------|------------|-------------|
| Haiku | Quick Q&A, formatting | $0.25/M | $1.25/M |
| Sonnet | Implementation, analysis | $3/M | $15/M |
| Opus | Architecture, security | $15/M | $75/M |

Default model: **Opus** (configurable via `CLAUDEBOT_MODEL`)

---

## Development Circle Personas

| Phase | Persona | Role | Style |
|-------|---------|------|-------|
| 1 | Graydon | Implementation | Jon Gjengset - idiomatic Rust |
| 2 | Linus | Code Review | Constructive Torvalds |
| 3 | Maria | Testing | Comprehensive edge cases |
| 4 | Kai | Optimization | HFT, zero-allocation |
| 5 | Sentinel | Security Audit | OWASP, red team |

---

## Troubleshooting

### "MCP server not found"
- Verify the binary path in `config.json` is absolute
- Check the binary exists and is executable: `ls -la /path/to/claudebot-mcp`

### "ANTHROPIC_API_KEY not set"
- Ensure `.env` file exists in the working directory, OR
- Set the env var in Claude Code config, OR
- Export it: `export ANTHROPIC_API_KEY=sk-ant-...`

### "Connection refused" (Ollama)
- Ollama is optional - the router works without it
- If using Ollama, ensure it's running: `ollama serve`

### Check logs
```bash
# MCP servers log to stderr
# Run manually to see logs:
ANTHROPIC_API_KEY=sk-ant-xxx ./target/release/claudebot-mcp 2>&1
```

### Rebuild after changes
```bash
cargo build --release
# Restart Claude Code to reload the MCP server
```

---

## Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_memory
```

---

## Telegram Bot Mode

Run the bot as a Telegram interface with conversation memory:

```bash
# Start Telegram bot
claudebot-mcp --telegram
```

### Configuration

```bash
# .env
TELEGRAM_BOT_TOKEN=your-bot-token
TELEGRAM_ALLOWED_USERS=123456789,987654321

# Conversation memory
CONVERSATION_DB_PATH=/home/eliot/workspace/conversations.db
CONVERSATION_MAX_MESSAGES=50
CONVERSATION_TTL=604800  # 7 days
```

### Commands

| Command | Description |
|---------|-------------|
| `/circle <task>` | Run Development Circle pipeline |
| `/memory` | Show conversation stats |
| `/history` | Show recent messages |
| `/clear` | Clear conversation history |
| `/context` | Load context from file |
| `/bypass <task>` | Execute on remote AR server (admin only) |

### Conversation Memory

The bot remembers context between messages:
- Rolling window of 50 messages per chat
- 7-day TTL for automatic cleanup
- Multi-chat isolation

See [docs/MEMORY.md](docs/MEMORY.md) for details.

---

## Bypass Bridge Mode

Run the bridge server for remote Claude Code execution:

```bash
# On AR server
claudebot-mcp --bridge --port 9999
```

### Architecture

```
Hetzner (Telegram)    ───HTTP───►    AR (Bridge)
     │                                   │
     │  /bypass "fix nginx"              │
     │                                   │
     └───────────────────────────────────┘
                                         │
                                         ▼
                              Claude Code CLI
                              (--dangerously-skip-permissions)
```

### Configuration

**AR Server (Bridge):**
```bash
BRIDGE_PORT=9999
BRIDGE_API_KEY=shared-secret-key
BRIDGE_WORKING_DIR=/tmp/claudebot
BRIDGE_TIMEOUT=300
BRIDGE_RATE_LIMIT=10
BRIDGE_ALLOWED_ADMINS=123456789
```

**Hetzner (Client):**
```bash
BRIDGE_URL=http://10.0.0.1:9999
BRIDGE_API_KEY=shared-secret-key
BRIDGE_TIMEOUT=300
```

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check (no auth) |
| GET | `/status` | Server status |
| POST | `/execute` | Execute Claude task |
| POST | `/file/read` | Read file from AR |

See [docs/BYPASS.md](docs/BYPASS.md) for full documentation.

---

## License

MIT
