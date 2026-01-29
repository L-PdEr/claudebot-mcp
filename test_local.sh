#!/bin/bash
# Local Testing Script for ClaudeBotMCP
# Tests each component independently

set -e
cd "$(dirname "$0")"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_ok() { echo -e "${GREEN}✓${NC} $1"; }
log_fail() { echo -e "${RED}✗${NC} $1"; }
log_warn() { echo -e "${YELLOW}!${NC} $1"; }
log_info() { echo -e "  $1"; }

echo "========================================"
echo "  ClaudeBotMCP Component Tests"
echo "========================================"
echo ""

# 1. Check binary exists
echo "1. Binary"
if [ -f target/release/claudebot-mcp ]; then
    log_ok "Release binary exists ($(du -h target/release/claudebot-mcp | cut -f1))"
else
    log_warn "Release binary missing, building..."
    cargo build --release
fi

# 2. Run unit tests
echo ""
echo "2. Unit Tests"
if cargo test --quiet 2>/dev/null; then
    TEST_COUNT=$(cargo test 2>&1 | grep -E "^test result" | head -1 | grep -oE "[0-9]+ passed" | head -1)
    log_ok "All tests passing ($TEST_COUNT)"
else
    log_fail "Some tests failed"
fi

# 3. Check Claude CLI
echo ""
echo "3. Claude CLI"
if command -v claude &> /dev/null; then
    CLAUDE_VERSION=$(claude --version 2>/dev/null | head -1 || echo "unknown")
    log_ok "Claude CLI found: $CLAUDE_VERSION"
else
    log_fail "Claude CLI not found (required for bridge)"
fi

# 4. Check Ollama
echo ""
echo "4. Ollama (for smart routing)"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
if curl -s "$OLLAMA_URL/api/tags" &>/dev/null; then
    MODELS=$(curl -s "$OLLAMA_URL/api/tags" | jq -r '.models[].name' 2>/dev/null | tr '\n' ', ' | sed 's/,$//')
    log_ok "Ollama running at $OLLAMA_URL"
    log_info "Models: $MODELS"
else
    log_warn "Ollama not running at $OLLAMA_URL (routing will use fallback)"
fi

# 5. Check environment variables
echo ""
echo "5. Environment Variables"

check_env() {
    if [ -n "${!1}" ]; then
        log_ok "$1 is set"
    else
        log_warn "$1 not set"
    fi
}

check_env "TELOXIDE_TOKEN"
check_env "ANTHROPIC_API_KEY"
check_env "BRIDGE_API_KEY"

# 6. gRPC Server Test (if running)
echo ""
echo "6. gRPC Bridge"
GRPC_PORT="${BRIDGE_GRPC_PORT:-9998}"
if nc -z localhost $GRPC_PORT 2>/dev/null; then
    log_ok "gRPC server running on port $GRPC_PORT"
else
    log_warn "gRPC server not running on port $GRPC_PORT"
    log_info "Start with: cargo run -- grpc-server"
fi

# 7. Database paths
echo ""
echo "7. Database Paths"
DB_DIR="${HOME}/.claudebot"
mkdir -p "$DB_DIR" 2>/dev/null
if [ -w "$DB_DIR" ]; then
    log_ok "Database directory writable: $DB_DIR"
else
    log_fail "Cannot write to $DB_DIR"
fi

echo ""
echo "========================================"
echo "  Quick Start Commands"
echo "========================================"
echo ""
echo "# Terminal 1: Start gRPC server (for bypass)"
echo "export BRIDGE_API_KEY=test-key"
echo "cargo run -- grpc-server"
echo ""
echo "# Terminal 2: Start Telegram bot"
echo "export TELOXIDE_TOKEN=your_token"
echo "export ANTHROPIC_API_KEY=your_key"
echo "export BRIDGE_GRPC_URL=http://localhost:9998"
echo "export BRIDGE_API_KEY=test-key"
echo "cargo run -- telegram"
echo ""
echo "# Test commands in Telegram:"
echo "/start - Check bot responds"
echo "/help - See all commands"
echo "/bypass_status - Check bridge connection"
echo "/bypass echo hello - Test bridge execution"
echo ""
