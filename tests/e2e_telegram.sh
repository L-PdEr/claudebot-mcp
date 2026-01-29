#!/bin/bash
#
# End-to-End Test for ClaudeBot Telegram
# Tests the full flow: bot startup -> message receiving -> response
#
set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration (override with environment variables)
BOT_TOKEN="${TELEGRAM_BOT_TOKEN:-}"
USER_ID="${TELEGRAM_USER_ID:-}"
PROJECT_DIR="/home/eliot/personal/dev/quantum-nexus-trading/claudebot-mcp"
TEST_DURATION="${TEST_DURATION:-20}"

echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}       ClaudeBot Telegram End-to-End Test${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo ""

# Check required environment variables
if [ -z "$BOT_TOKEN" ]; then
    echo -e "${RED}ERROR: TELEGRAM_BOT_TOKEN not set${NC}"
    echo "Export it: export TELEGRAM_BOT_TOKEN=your_token"
    exit 1
fi

if [ -z "$USER_ID" ]; then
    echo -e "${YELLOW}WARNING: TELEGRAM_USER_ID not set (bot will accept all users)${NC}"
fi

echo -e "${YELLOW}[1/7] Verifying Telegram Bot Token...${NC}"
BOT_INFO=$(curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getMe")
if echo "$BOT_INFO" | grep -q '"ok":true'; then
    BOT_USERNAME=$(echo "$BOT_INFO" | grep -o '"username":"[^"]*"' | cut -d'"' -f4)
    BOT_ID=$(echo "$BOT_INFO" | grep -o '"id":[0-9]*' | head -1 | cut -d':' -f2)
    echo -e "${GREEN}✓ Bot verified: @${BOT_USERNAME} (ID: ${BOT_ID})${NC}"
else
    echo -e "${RED}ERROR: Invalid bot token${NC}"
    echo "$BOT_INFO"
    exit 1
fi

echo ""
echo -e "${YELLOW}[2/7] Checking webhook status...${NC}"
WEBHOOK_INFO=$(curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getWebhookInfo")
WEBHOOK_URL=$(echo "$WEBHOOK_INFO" | grep -o '"url":"[^"]*"' | cut -d'"' -f4)
if [ -z "$WEBHOOK_URL" ] || [ "$WEBHOOK_URL" == "" ]; then
    echo -e "${GREEN}✓ No webhook set (polling mode OK)${NC}"
else
    echo -e "${YELLOW}Webhook detected: $WEBHOOK_URL${NC}"
    echo -e "${YELLOW}Removing webhook for polling mode...${NC}"
    curl -s "https://api.telegram.org/bot${BOT_TOKEN}/deleteWebhook" > /dev/null
    echo -e "${GREEN}✓ Webhook removed${NC}"
fi

echo ""
echo -e "${YELLOW}[3/7] Checking pending updates...${NC}"
PENDING=$(echo "$WEBHOOK_INFO" | grep -o '"pending_update_count":[0-9]*' | cut -d':' -f2)
echo -e "${GREEN}✓ Pending updates: ${PENDING:-0}${NC}"

echo ""
echo -e "${YELLOW}[4/7] Flushing old updates...${NC}"
# Get and discard any pending updates to start fresh
curl -s "https://api.telegram.org/bot${BOT_TOKEN}/getUpdates?offset=-1" > /dev/null
echo -e "${GREEN}✓ Updates flushed${NC}"

echo ""
echo -e "${YELLOW}[5/7] Building bot (release)...${NC}"
cd "$PROJECT_DIR"
if cargo build --release 2>&1 | tail -3; then
    echo -e "${GREEN}✓ Build complete${NC}"
else
    echo -e "${RED}Build failed!${NC}"
    exit 1
fi

echo ""
echo -e "${YELLOW}[6/7] Creating test environment...${NC}"
TEST_DIR="/tmp/claudebot-e2e-test-$$"
mkdir -p "$TEST_DIR/workspace"

# Create .env file in test directory
cat > "$TEST_DIR/.env" << EOF
TELEGRAM_BOT_TOKEN=$BOT_TOKEN
TELEGRAM_ALLOWED_USERS=${USER_ID}
CLAUDE_WORKING_DIR=$TEST_DIR/workspace
USAGE_DB_PATH=$TEST_DIR/usage.db
MEMORY_DB_PATH=$TEST_DIR/memory.db
EOF

echo -e "${GREEN}✓ Test environment: $TEST_DIR${NC}"

echo ""
echo -e "${YELLOW}[7/7] Running bot (${TEST_DURATION} seconds)...${NC}"
echo ""
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  SEND A MESSAGE TO @${BOT_USERNAME} NOW!${NC}"
echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
echo ""
echo "--- Bot Output ---"

# Run bot with timeout, capturing output
cd "$TEST_DIR"
set +e  # Don't exit on error (timeout returns non-zero)

# Export env vars for the bot
export TELEGRAM_BOT_TOKEN="$BOT_TOKEN"
export TELEGRAM_ALLOWED_USERS="${USER_ID}"
export CLAUDE_WORKING_DIR="$TEST_DIR/workspace"
export USAGE_DB_PATH="$TEST_DIR/usage.db"
export MEMORY_DB_PATH="$TEST_DIR/memory.db"
export RUST_LOG="info"

timeout "$TEST_DURATION" "$PROJECT_DIR/target/release/claudebot-mcp" --telegram 2>&1
EXIT_CODE=$?
set -e

echo ""
echo "--- End Bot Output ---"
echo ""

# Analyze results
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}                    TEST RESULTS${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo ""

if [ $EXIT_CODE -eq 124 ]; then
    echo -e "${GREEN}✓ Bot ran for ${TEST_DURATION} seconds (timeout as expected)${NC}"
else
    echo -e "${YELLOW}⚠ Bot exited with code: $EXIT_CODE${NC}"
fi

echo ""
echo "Check the output above for:"
echo "  ✓ 'Bot authenticated: @${BOT_USERNAME}' - Token valid"
echo "  ✓ 'Bot is now LIVE' - Dispatcher started"
echo "  ✓ '>>> Message received' - Your message was processed"
echo ""
echo "If messages weren't received:"
echo "  1. Make sure you sent a message during the test"
echo "  2. Verify your user ID is in TELEGRAM_ALLOWED_USERS"
echo "     (Send /start to @userinfobot to find your ID)"
echo "  3. Check no other bot instance is running"
echo ""

# Cleanup
rm -rf "$TEST_DIR"
echo -e "${GREEN}✓ Cleanup complete${NC}"
