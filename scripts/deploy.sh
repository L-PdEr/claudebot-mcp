#!/bin/bash
# Deploy ClaudeBot to production server
# Usage: ./scripts/deploy.sh

set -e

SERVER="root@clawdbot-prod"
BINARY="target/release/claudebot-mcp"
REMOTE_PATH="/home/eliot/bin/claudebot-mcp"
REMOTE_WORKSPACE="/home/eliot/workspace"
SERVICE="claudebot-telegram"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== ClaudeBot Deploy Script ==="

# Step 1: Build locally
echo "[1/6] Building release..."
cargo build --release

# Step 2: Check and install dependencies on server
echo "[2/6] Checking server dependencies..."
ssh "$SERVER" << 'DEPS_EOF'
    echo "Checking dependencies..."

    # Check Claude CLI
    if ! command -v claude &> /dev/null; then
        echo "Installing Claude CLI..."
        if command -v npm &> /dev/null; then
            npm install -g @anthropic-ai/claude-code
        else
            echo "ERROR: npm not found. Install Node.js first:"
            echo "  curl -fsSL https://deb.nodesource.com/setup_20.x | bash -"
            echo "  apt-get install -y nodejs"
            exit 1
        fi
    else
        echo "✓ Claude CLI installed"
    fi

    # Check Ollama
    if ! command -v ollama &> /dev/null; then
        echo "Installing Ollama..."
        curl -fsSL https://ollama.com/install.sh | sh
        systemctl enable ollama
        systemctl start ollama
        sleep 5
        # Pull required models
        ollama pull nomic-embed-text
        ollama pull llama3.2
    else
        echo "✓ Ollama installed"
        # Ensure models are available
        ollama list | grep -q nomic-embed-text || ollama pull nomic-embed-text
        ollama list | grep -q llama3.2 || ollama pull llama3.2
    fi

    # Check SQLite
    if ! command -v sqlite3 &> /dev/null; then
        echo "Installing SQLite..."
        apt-get update && apt-get install -y sqlite3
    else
        echo "✓ SQLite installed"
    fi

    # Ensure directories exist
    mkdir -p /home/eliot/bin /home/eliot/workspace /home/eliot/.config/claudebot
    chown -R eliot:eliot /home/eliot/bin /home/eliot/workspace /home/eliot/.config/claudebot

    echo "Dependencies OK"
DEPS_EOF

# Step 3: Copy binary
echo "[3/6] Copying binary to server..."
scp "$BINARY" "$SERVER:/tmp/claudebot-mcp.new"

# Step 4: Copy config files
echo "[4/6] Copying config files..."
scp "$PROJECT_DIR/configs/CLAUDE.md" "$SERVER:$REMOTE_WORKSPACE/CLAUDE.md"
ssh "$SERVER" "chown eliot:eliot $REMOTE_WORKSPACE/CLAUDE.md"

# Step 5: Stop all instances and deploy
echo "[5/6] Stopping services and deploying..."
ssh "$SERVER" << 'DEPLOY_EOF'
    # Stop system service
    systemctl stop claudebot-telegram 2>/dev/null || true

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

    # Deploy new binary
    cp /tmp/claudebot-mcp.new /home/eliot/bin/claudebot-mcp
    chown eliot:eliot /home/eliot/bin/claudebot-mcp
    chmod +x /home/eliot/bin/claudebot-mcp
    rm /tmp/claudebot-mcp.new

    echo "Binary deployed"
DEPLOY_EOF

# Step 6: Start service
echo "[6/6] Starting service..."
ssh "$SERVER" << 'START_EOF'
    systemctl start claudebot-telegram
    sleep 2

    # Verify
    if systemctl is-active --quiet claudebot-telegram; then
        echo "✓ Service started successfully"
        systemctl status claudebot-telegram | head -10
    else
        echo "✗ Service failed to start"
        journalctl -u claudebot-telegram -n 20 --no-pager
        exit 1
    fi

    # Check for duplicate processes
    COUNT=$(pgrep -c -f "claudebot-mcp --telegram" 2>/dev/null || echo 0)
    if [ "$COUNT" -eq 1 ]; then
        echo "✓ Single instance running"
    else
        echo "WARNING: $COUNT instances running"
    fi
START_EOF

echo ""
echo "=== Deploy Complete ==="
echo "Config: $REMOTE_WORKSPACE/CLAUDE.md"
echo "Binary: $REMOTE_PATH"
