#!/bin/bash
#
# ClaudeBot Telegram Deployment Script
# Deploys Eliot Brain to Hetzner server via Tailscale
#
set -e

# Configuration
REMOTE_USER="eliot"
REMOTE_HOST="100.94.120.80"  # Hetzner Tailscale IP
LOCAL_PROJECT="/home/eliot/personal/dev/claudebot-mcp"

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
echo "[3/9] Creating remote directories..."
ssh "$REMOTE_USER@$REMOTE_HOST" << 'ENDSSH'
mkdir -p ~/bin
mkdir -p ~/workspace
mkdir -p ~/.config/systemd/user
ENDSSH
echo "✓ Directories created"

# Install dependencies on server
echo ""
echo "[4/9] Installing dependencies on server..."
ssh "$REMOTE_USER@$REMOTE_HOST" << 'ENDSSH'
echo "Checking and installing required tools..."

# Install Rust/Cargo if missing
if ! command -v cargo &> /dev/null; then
    echo "  Installing Rust/Cargo..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source ~/.cargo/env
    echo "  ✓ Rust installed: $(cargo --version)"
else
    echo "  ✓ Cargo already installed: $(cargo --version)"
fi

# Install Node.js if missing (needed for Claude CLI)
if ! command -v node &> /dev/null; then
    echo "  Installing Node.js via nvm..."
    curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.0/install.sh | bash
    export NVM_DIR="$HOME/.nvm"
    [ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh"
    nvm install --lts
    echo "  ✓ Node.js installed: $(node --version)"
else
    echo "  ✓ Node.js already installed: $(node --version)"
fi

# Install Claude CLI if missing
if ! command -v claude &> /dev/null; then
    echo "  Installing Claude CLI..."
    npm install -g @anthropic-ai/claude-code
    echo "  ✓ Claude CLI installed: $(claude --version)"
else
    echo "  ✓ Claude CLI already installed: $(claude --version)"
fi

# Install gh CLI if missing (optional but useful)
if ! command -v gh &> /dev/null; then
    echo "  Installing GitHub CLI..."
    (type -p wget >/dev/null || (sudo apt update && sudo apt-get install wget -y)) \
    && sudo mkdir -p -m 755 /etc/apt/keyrings \
    && out=$(mktemp) && wget -nv -O$out https://cli.github.com/packages/githubcli-archive-keyring.gpg \
    && cat $out | sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null \
    && sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && sudo apt update \
    && sudo apt install gh -y 2>/dev/null || echo "  (gh install skipped - manual install may be needed)"
else
    echo "  ✓ GitHub CLI already installed: $(gh --version | head -1)"
fi

echo "✓ Dependencies checked"
ENDSSH
echo "✓ Server dependencies installed"

# Stop service before copying (binary may be in use)
echo ""
echo "[5/10] Stopping service for binary update..."
ssh "$REMOTE_USER@$REMOTE_HOST" "systemctl --user stop claudebot-telegram.service 2>/dev/null || true; rm -f ~/bin/claudebot-mcp"
echo "✓ Service stopped"

# Copy binary
echo ""
echo "[6/10] Copying binary to server..."
scp "$LOCAL_PROJECT/target/release/claudebot-mcp" "$REMOTE_USER@$REMOTE_HOST:~/bin/"
ssh "$REMOTE_USER@$REMOTE_HOST" "chmod +x ~/bin/claudebot-mcp"
echo "✓ Binary deployed"

# Copy workspace files (CLAUDE.md, etc.)
echo ""
echo "[7/10] Copying workspace files..."
rsync -av --progress "$LOCAL_PROJECT/workspace/" "$REMOTE_USER@$REMOTE_HOST:~/workspace/"
echo "✓ Workspace files synced"

# Copy Claude Code commands (circle, security, review skills)
echo ""
echo "[8/10] Copying Claude Code commands..."
ssh "$REMOTE_USER@$REMOTE_HOST" "mkdir -p ~/workspace/.claude/commands"
rsync -av --progress "$LOCAL_PROJECT/.claude/" "$REMOTE_USER@$REMOTE_HOST:~/workspace/.claude/"
echo "✓ Claude Code commands synced (security, review, circle)"

# Copy systemd service
echo ""
echo "[9/10] Setting up systemd service..."
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
echo "[10/10] Starting service..."
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
