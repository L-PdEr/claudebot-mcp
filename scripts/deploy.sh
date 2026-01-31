#!/bin/bash
# Deploy ClaudeBot to production server
# Usage: ./scripts/deploy.sh

set -e

SERVER="root@clawdbot-prod"
BINARY="target/release/claudebot-mcp"
REMOTE_PATH="/home/eliot/bin/claudebot-mcp"
SERVICE="claudebot-telegram"

echo "=== ClaudeBot Deploy Script ==="

# Build
echo "[1/5] Building release..."
cargo build --release

# Copy binary
echo "[2/5] Copying binary to server..."
scp "$BINARY" "$SERVER:/tmp/claudebot-mcp.new"

# Stop all instances and deploy
echo "[3/5] Stopping services and killing all instances..."
ssh "$SERVER" bash -c '
    # Stop system service
    systemctl stop '"$SERVICE"' 2>/dev/null || true

    # Stop and disable user services (eliot)
    XDG_RUNTIME_DIR=/run/user/$(id -u eliot) runuser -u eliot -- systemctl --user stop claudebot-telegram claudebot-grpc 2>/dev/null || true
    XDG_RUNTIME_DIR=/run/user/$(id -u eliot) runuser -u eliot -- systemctl --user disable claudebot-telegram claudebot-grpc 2>/dev/null || true

    # Kill any remaining processes
    pkill -9 -f "claudebot-mcp" 2>/dev/null || true
    sleep 2

    # Verify all killed
    if pgrep -f "claudebot-mcp" > /dev/null; then
        echo "WARNING: Some processes still running, force killing..."
        pkill -9 -f "claudebot-mcp" 2>/dev/null || true
        sleep 1
    fi
'

# Deploy new binary
echo "[4/5] Deploying new binary..."
ssh "$SERVER" bash -c '
    cp /tmp/claudebot-mcp.new '"$REMOTE_PATH"'
    chown eliot:eliot '"$REMOTE_PATH"'
    chmod +x '"$REMOTE_PATH"'
    rm /tmp/claudebot-mcp.new
'

# Start service
echo "[5/5] Starting service..."
ssh "$SERVER" bash -c '
    systemctl start '"$SERVICE"'
    sleep 2

    # Verify
    if systemctl is-active --quiet '"$SERVICE"'; then
        echo "✓ Service started successfully"
        systemctl status '"$SERVICE"' | head -10
    else
        echo "✗ Service failed to start"
        journalctl -u '"$SERVICE"' -n 20 --no-pager
        exit 1
    fi

    # Check for duplicate processes
    COUNT=$(pgrep -c -f "claudebot-mcp --telegram" 2>/dev/null || echo 0)
    if [ "$COUNT" -eq 1 ]; then
        echo "✓ Single instance running"
    else
        echo "⚠ WARNING: $COUNT instances running"
    fi
'

echo ""
echo "=== Deploy Complete ==="
