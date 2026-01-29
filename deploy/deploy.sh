#!/bin/bash
#
# ClaudeBot Telegram Deployment Script
# Deploys Eliot Brain to Hetzner server via Tailscale
#
set -e

# Configuration
REMOTE_USER="eliot"
REMOTE_HOST="100.94.120.80"  # Hetzner Tailscale IP
LOCAL_PROJECT="/home/eliot/personal/dev/quantum-nexus-trading/claudebot-mcp"

echo "╔═══════════════════════════════════════════════════════════╗"
echo "║           CLAUDEBOT TELEGRAM DEPLOYMENT                   ║"
echo "║              Eliot Brain → Hetzner                        ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""

# Check Tailscale connection
echo "[1/7] Checking Tailscale connection..."
if ! ping -c 1 -W 2 "$REMOTE_HOST" &>/dev/null; then
    echo "ERROR: Cannot reach Hetzner at $REMOTE_HOST"
    echo "Make sure Tailscale is connected: tailscale status"
    exit 1
fi
echo "✓ Hetzner reachable via Tailscale"

# Build release binary
echo ""
echo "[2/7] Building release binary..."
cd "$LOCAL_PROJECT"
cargo build --release
echo "✓ Binary built: target/release/claudebot-mcp"

# Create remote directories
echo ""
echo "[3/7] Creating remote directories..."
ssh "$REMOTE_USER@$REMOTE_HOST" << 'ENDSSH'
mkdir -p ~/bin
mkdir -p ~/workspace
mkdir -p ~/.config/systemd/user
ENDSSH
echo "✓ Directories created"

# Copy binary
echo ""
echo "[4/7] Copying binary to server..."
scp "$LOCAL_PROJECT/target/release/claudebot-mcp" "$REMOTE_USER@$REMOTE_HOST:~/bin/"
ssh "$REMOTE_USER@$REMOTE_HOST" "chmod +x ~/bin/claudebot-mcp"
echo "✓ Binary deployed"

# Copy workspace files (CLAUDE.md, etc.)
echo ""
echo "[5/7] Copying workspace files..."
rsync -av --progress "$LOCAL_PROJECT/workspace/" "$REMOTE_USER@$REMOTE_HOST:~/workspace/"
echo "✓ Workspace files synced"

# Copy systemd service
echo ""
echo "[6/7] Setting up systemd service..."
scp "$LOCAL_PROJECT/deploy/claudebot-telegram.service" "$REMOTE_USER@$REMOTE_HOST:~/.config/systemd/user/"

# Check if env file exists, if not copy template
ssh "$REMOTE_USER@$REMOTE_HOST" << 'ENDSSH'
if [ ! -f ~/.env.claudebot ]; then
    echo "WARNING: ~/.env.claudebot not found!"
    echo "Please create it with your TELEGRAM_BOT_TOKEN and ANTHROPIC_API_KEY"
fi
ENDSSH
echo "✓ Systemd service installed"

# Enable and start service
echo ""
echo "[7/7] Starting service..."
ssh "$REMOTE_USER@$REMOTE_HOST" << 'ENDSSH'
# Enable lingering for user services
loginctl enable-linger eliot 2>/dev/null || true

# Reload and restart
systemctl --user daemon-reload
systemctl --user enable claudebot-telegram.service
systemctl --user restart claudebot-telegram.service

# Check status
sleep 2
systemctl --user status claudebot-telegram.service --no-pager || true
ENDSSH

echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║                 DEPLOYMENT COMPLETE                       ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "Commands:"
echo "  Check logs:    ssh $REMOTE_USER@$REMOTE_HOST 'journalctl --user -u claudebot-telegram -f'"
echo "  Restart:       ssh $REMOTE_USER@$REMOTE_HOST 'systemctl --user restart claudebot-telegram'"
echo "  Stop:          ssh $REMOTE_USER@$REMOTE_HOST 'systemctl --user stop claudebot-telegram'"
echo ""
echo "If service fails, check:"
echo "  1. ~/.env.claudebot has TELEGRAM_BOT_TOKEN and ANTHROPIC_API_KEY"
echo "  2. Claude CLI is installed: npm install -g @anthropic-ai/claude-code"
echo ""
