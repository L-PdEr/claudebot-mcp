#!/bin/bash
# Deploy ClaudeBot to Hetzner server
# Usage: ./scripts/deploy-hetzner.sh

set -e

REMOTE_HOST="eliot@100.94.120.80"
REMOTE_BIN="/home/eliot/bin/claudebot-mcp"
REMOTE_ENV="/home/eliot/.config/claudebot/env"
LOCAL_BIN="target/release/claudebot-mcp"

echo "=== ClaudeBot Hetzner Deployment ==="

# Step 1: Build release binary
echo "[1/6] Building release binary..."
cargo build --release

# Step 2: Copy binary to server
echo "[2/6] Copying binary to server..."
scp "$LOCAL_BIN" "${REMOTE_HOST}:${REMOTE_BIN}.new"

# Step 3: Stop old bot
echo "[3/6] Stopping old bot..."
ssh "$REMOTE_HOST" "pkill -f 'claudebot-mcp --telegram' || true"
sleep 1

# Step 4: Replace binary
echo "[4/6] Replacing binary..."
ssh "$REMOTE_HOST" "mv ${REMOTE_BIN}.new ${REMOTE_BIN} && chmod +x ${REMOTE_BIN}"

# Step 5: Ensure Ollama is running
echo "[5/6] Checking Ollama..."
ssh "$REMOTE_HOST" "
    if ! curl -s http://localhost:11434/api/tags > /dev/null 2>&1; then
        echo 'Starting Ollama...'
        nohup ollama serve > /tmp/ollama.log 2>&1 &
        sleep 3
    fi

    # Ensure required models are available
    for model in mxbai-embed-large llama3.2; do
        if ! ollama list | grep -q \"\$model\"; then
            echo \"Pulling \$model...\"
            ollama pull \$model
        fi
    done

    echo 'Ollama models:'
    ollama list
"

# Step 6: Start new bot
echo "[6/6] Starting new bot..."
ssh "$REMOTE_HOST" "
    # Load environment from secure file
    if [ -f ${REMOTE_ENV} ]; then
        set -a
        source ${REMOTE_ENV}
        set +a
    else
        echo 'ERROR: Environment file not found at ${REMOTE_ENV}'
        exit 1
    fi

    cd ~
    nohup ${REMOTE_BIN} --telegram > /tmp/claudebot-telegram.log 2>&1 &
    sleep 3

    if pgrep -f 'claudebot-mcp --telegram' > /dev/null; then
        echo 'Bot started successfully!'
        pgrep -a -f 'claudebot-mcp --telegram'
    else
        echo 'ERROR: Bot failed to start. Check logs:'
        tail -20 /tmp/claudebot-telegram.log
        exit 1
    fi
"

echo ""
echo "=== Deployment Complete ==="
echo "Bot is running on Hetzner with:"
echo "  - Embedding model: mxbai-embed-large (1024d)"
echo "  - Memory: RRF + time-decay + reranking"
echo "  - Embedding cache: 1000 entries, 1hr TTL"
