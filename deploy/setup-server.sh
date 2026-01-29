#!/bin/bash
#
# Server Setup Script for ClaudeBot
# Run this on Hetzner server before first deployment
#
set -e

echo "╔═══════════════════════════════════════════════════════════╗"
echo "║           HETZNER SERVER SETUP                            ║"
echo "║           Prerequisites for Eliot Brain                   ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""

# Update system
echo "[1/6] Updating system..."
sudo apt update && sudo apt upgrade -y
echo "✓ System updated"

# Install Node.js (for Claude CLI)
echo ""
echo "[2/6] Installing Node.js..."
if ! command -v node &>/dev/null; then
    curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
    sudo apt install -y nodejs
fi
echo "✓ Node.js $(node --version)"

# Install Claude Code CLI
echo ""
echo "[3/6] Installing Claude Code CLI..."
if ! command -v claude &>/dev/null; then
    sudo npm install -g @anthropic-ai/claude-code
fi
echo "✓ Claude CLI $(claude --version 2>/dev/null | head -1 || echo 'installed')"

# Create directories
echo ""
echo "[4/6] Creating directories..."
mkdir -p ~/bin
mkdir -p ~/workspace
mkdir -p ~/.config/systemd/user
echo "✓ Directories created"

# Setup environment file
echo ""
echo "[5/6] Setting up environment..."
if [ ! -f ~/.env.claudebot ]; then
    cat > ~/.env.claudebot << 'EOF'
# ClaudeBot Environment - EDIT THESE VALUES!
TELEGRAM_BOT_TOKEN=your_token_here
TELEGRAM_ALLOWED_USERS=
ANTHROPIC_API_KEY=your_api_key_here
CLAUDE_WORKING_DIR=/home/eliot/workspace
EOF
    echo "Created ~/.env.claudebot - PLEASE EDIT WITH YOUR TOKENS!"
else
    echo "~/.env.claudebot already exists"
fi

# Enable user systemd lingering
echo ""
echo "[6/6] Enabling systemd user services..."
sudo loginctl enable-linger eliot
echo "✓ User services enabled"

echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║                 SETUP COMPLETE                            ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""
echo "NEXT STEPS:"
echo "1. Edit ~/.env.claudebot with your actual tokens:"
echo "   nano ~/.env.claudebot"
echo ""
echo "2. Authenticate Claude CLI:"
echo "   claude auth"
echo ""
echo "3. Run deployment from your local machine:"
echo "   ./deploy.sh"
echo ""
