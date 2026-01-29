# Bypass Bridge Documentation

The Bypass Bridge enables remote Claude Code execution from the Telegram bot running on Hetzner to the AR server where Claude CLI is installed.

## Architecture

```
┌─────────────────┐      HTTPS       ┌─────────────────┐
│  Hetzner VPS    │  ────────────>   │    AR Server    │
│  (Telegram Bot) │                  │  (Claude CLI)   │
│                 │                  │                 │
│  BridgeClient   │  ◄────────────   │  BridgeServer   │
│                 │    Response      │                 │
└─────────────────┘                  └─────────────────┘
```

## Components

### BridgeServer (AR)

HTTP server listening on port 9999 (configurable) that:
- Authenticates requests via Bearer token
- Rate limits requests per chat (10/min default)
- Restricts access to admin user IDs
- Executes Claude CLI with task prompts
- Returns structured JSON responses
- Supports session resumption

### BridgeClient (Hetzner)

HTTP client that:
- Connects to BridgeServer via HTTPS
- Handles authentication
- Sends task execution requests
- Supports file read operations

## Configuration

### Environment Variables (Server)

| Variable | Default | Description |
|----------|---------|-------------|
| `BRIDGE_PORT` | `9999` | Server listen port |
| `BRIDGE_API_KEY` | Required | Authentication token |
| `BRIDGE_WORKING_DIR` | `/tmp/claudebot` | Base working directory |
| `BRIDGE_TIMEOUT` | `300` | Max execution time (seconds) |
| `BRIDGE_RATE_LIMIT` | `10` | Requests per minute per chat |
| `BRIDGE_ALLOWED_ADMINS` | Empty | Comma-separated Telegram user IDs |

### Environment Variables (Client)

| Variable | Default | Description |
|----------|---------|-------------|
| `BRIDGE_URL` | Required | Server URL (e.g., `http://ar.local:9999`) |
| `BRIDGE_API_KEY` | Required | Same token as server |
| `BRIDGE_TIMEOUT` | `300` | Request timeout (seconds) |

## API Endpoints

### POST /execute

Execute a Claude Code task.

**Request:**
```json
{
  "task": "Describe what's in the current directory",
  "chat_id": 12345,
  "session_id": "optional-session-id",
  "working_dir": "/optional/path",
  "autonomous": true
}
```

**Response:**
```json
{
  "success": true,
  "text": "The directory contains...",
  "session_id": "new-session-id",
  "duration_ms": 2500,
  "cost_usd": 0.015
}
```

### POST /file/read

Read a file from the AR server.

**Request:**
```json
{
  "path": "/absolute/path/to/file",
  "analyze": false,
  "max_bytes": 10240
}
```

**Response:**
```json
{
  "success": true,
  "content": "file contents...",
  "file_size": 1024,
  "truncated": false
}
```

### GET /health

Health check (no authentication required).

### GET /status

Server status (requires authentication).

## Telegram Commands

| Command | Description |
|---------|-------------|
| `/bypass <task>` | Execute task via AR |
| `/bypass_status` | Check bridge connection |
| `/bypass_file <path>` | Read file with Claude analysis |
| `/bypass_cat <path>` | Read raw file content |

## Security Features

### Authentication
- Bearer token authentication on all endpoints except `/health`
- Token must match `BRIDGE_API_KEY` environment variable

### Rate Limiting
- Per-chat rate limiting prevents abuse
- Configurable requests per minute
- Automatic window reset

### Admin Whitelist
- Optional list of allowed Telegram user IDs
- Empty list allows all authenticated requests
- Non-whitelisted users receive 403 Forbidden

### Path Validation (File Operations)
- Requires absolute paths only
- Rejects directory traversal (`..`)
- Rejects null byte injection
- Validates file existence and type

## Running the Bridge

### On AR Server

```bash
export BRIDGE_API_KEY="your-secret-key"
export BRIDGE_ALLOWED_ADMINS="123456789,987654321"

cargo run --bin claudebot-mcp -- bridge-server
```

### On Hetzner (via Telegram Bot)

The bot automatically uses the bridge when configured:

```bash
export BRIDGE_URL="http://ar.example.com:9999"
export BRIDGE_API_KEY="your-secret-key"

cargo run --bin claudebot-mcp -- telegram
```

## Error Handling

| HTTP Status | Meaning |
|-------------|---------|
| 200 | Success (check `success` field in JSON) |
| 401 | Invalid or missing API key |
| 403 | Chat ID not in admin whitelist |
| 429 | Rate limit exceeded |
| 500 | Internal server error |

## Session Resumption

Claude Code sessions are tracked per chat ID. When a task completes:
1. The `session_id` is extracted from Claude's output
2. Stored in memory mapped to the `chat_id`
3. Automatically included in subsequent requests
4. Enables conversation continuity within Claude Code

## Timeout Handling

- Default timeout: 300 seconds (5 minutes)
- Configurable via `BRIDGE_TIMEOUT`
- Graceful timeout returns error response (not connection drop)
