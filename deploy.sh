#!/bin/bash
# ClaudeBotMCP Deployment Script
# Usage: ./deploy.sh [server|client|certs]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/claudebot-mcp"
DEPLOY_DIR="/opt/claudebot"
CERT_DIR="/etc/claudebot/certs"

case "${1:-help}" in
  certs)
    echo "Generating TLS certificates..."
    sudo mkdir -p "$CERT_DIR"

    # Generate CA
    sudo openssl genrsa -out "$CERT_DIR/ca.key" 4096
    sudo openssl req -x509 -new -nodes -key "$CERT_DIR/ca.key" \
      -sha256 -days 3650 -out "$CERT_DIR/ca.crt" \
      -subj "/CN=ClaudeBotCA/O=Velofi"

    # Generate server cert
    sudo openssl genrsa -out "$CERT_DIR/server.key" 2048
    sudo openssl req -new -key "$CERT_DIR/server.key" \
      -out "$CERT_DIR/server.csr" \
      -subj "/CN=claudebot-bridge/O=Velofi"
    sudo openssl x509 -req -in "$CERT_DIR/server.csr" \
      -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
      -CAcreateserial -out "$CERT_DIR/server.crt" \
      -days 365 -sha256

    sudo chmod 600 "$CERT_DIR"/*.key
    sudo chmod 644 "$CERT_DIR"/*.crt

    echo "Certificates generated in $CERT_DIR"
    ls -la "$CERT_DIR"
    ;;

  server)
    echo "Deploying gRPC Bridge Server (AR)..."

    # Build if needed
    if [ ! -f "$BINARY" ]; then
      echo "Building release binary..."
      cargo build --release
    fi

    # Create directories
    sudo mkdir -p "$DEPLOY_DIR"
    sudo mkdir -p /var/log/claudebot
    sudo mkdir -p /tmp/claudebot

    # Copy binary
    sudo cp "$BINARY" "$DEPLOY_DIR/"

    # Create systemd service
    sudo tee /etc/systemd/system/claudebot-bridge.service > /dev/null << 'EOF'
[Unit]
Description=ClaudeBot gRPC Bridge Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/tmp/claudebot
ExecStart=/opt/claudebot/claudebot-mcp grpc-server
Restart=always
RestartSec=5

# Environment
Environment=BRIDGE_GRPC_PORT=9998
Environment=BRIDGE_API_KEY=CHANGE_ME_SECRET_KEY
Environment=BRIDGE_WORKING_DIR=/tmp/claudebot
Environment=BRIDGE_TIMEOUT=300
Environment=BRIDGE_RATE_LIMIT=10
Environment=BRIDGE_TLS_CERT=/etc/claudebot/certs/server.crt
Environment=BRIDGE_TLS_KEY=/etc/claudebot/certs/server.key

# Logging
StandardOutput=append:/var/log/claudebot/bridge.log
StandardError=append:/var/log/claudebot/bridge-error.log

[Install]
WantedBy=multi-user.target
EOF

    echo ""
    echo "Edit /etc/systemd/system/claudebot-bridge.service to set:"
    echo "  - BRIDGE_API_KEY (secure random key)"
    echo "  - BRIDGE_ALLOWED_ADMINS (Telegram user IDs)"
    echo ""
    echo "Then run:"
    echo "  sudo systemctl daemon-reload"
    echo "  sudo systemctl enable claudebot-bridge"
    echo "  sudo systemctl start claudebot-bridge"
    ;;

  client)
    echo "Deploying Telegram Bot with gRPC Client (Hetzner)..."

    # Build if needed
    if [ ! -f "$BINARY" ]; then
      echo "Building release binary..."
      cargo build --release
    fi

    # Create directories
    sudo mkdir -p "$DEPLOY_DIR"
    sudo mkdir -p /var/log/claudebot
    sudo mkdir -p /home/claudebot/workspace

    # Copy binary
    sudo cp "$BINARY" "$DEPLOY_DIR/"

    # Create systemd service
    sudo tee /etc/systemd/system/claudebot-telegram.service > /dev/null << 'EOF'
[Unit]
Description=ClaudeBot Telegram Bot
After=network.target

[Service]
Type=simple
User=claudebot
WorkingDirectory=/home/claudebot/workspace
ExecStart=/opt/claudebot/claudebot-mcp telegram
Restart=always
RestartSec=5

# Telegram
Environment=TELOXIDE_TOKEN=YOUR_TELEGRAM_BOT_TOKEN
Environment=ALLOWED_USERS=

# gRPC Bridge
Environment=BRIDGE_GRPC_URL=https://ar.example.com:9998
Environment=BRIDGE_API_KEY=CHANGE_ME_SECRET_KEY
Environment=BRIDGE_CA_CERT=/etc/claudebot/certs/ca.crt
Environment=BRIDGE_TLS_DOMAIN=claudebot-bridge

# Claude
Environment=ANTHROPIC_API_KEY=YOUR_ANTHROPIC_KEY

# Ollama (optional)
Environment=OLLAMA_URL=http://localhost:11434

# Logging
StandardOutput=append:/var/log/claudebot/telegram.log
StandardError=append:/var/log/claudebot/telegram-error.log

[Install]
WantedBy=multi-user.target
EOF

    echo ""
    echo "Edit /etc/systemd/system/claudebot-telegram.service to set:"
    echo "  - TELOXIDE_TOKEN (from @BotFather)"
    echo "  - BRIDGE_GRPC_URL (AR server address)"
    echo "  - BRIDGE_API_KEY (same as server)"
    echo "  - ANTHROPIC_API_KEY"
    echo "  - ALLOWED_USERS (comma-separated Telegram IDs)"
    echo ""
    echo "Copy CA cert from server:"
    echo "  scp ar:/etc/claudebot/certs/ca.crt /etc/claudebot/certs/"
    echo ""
    echo "Then run:"
    echo "  sudo systemctl daemon-reload"
    echo "  sudo systemctl enable claudebot-telegram"
    echo "  sudo systemctl start claudebot-telegram"
    ;;

  status)
    echo "=== ClaudeBot Status ==="
    echo ""
    echo "Bridge Server:"
    systemctl status claudebot-bridge --no-pager 2>/dev/null || echo "  Not installed"
    echo ""
    echo "Telegram Bot:"
    systemctl status claudebot-telegram --no-pager 2>/dev/null || echo "  Not installed"
    echo ""
    echo "Logs:"
    echo "  Bridge: /var/log/claudebot/bridge.log"
    echo "  Telegram: /var/log/claudebot/telegram.log"
    ;;

  *)
    echo "ClaudeBotMCP Deployment"
    echo ""
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  certs   - Generate TLS certificates"
    echo "  server  - Deploy gRPC bridge server (run on AR)"
    echo "  client  - Deploy Telegram bot (run on Hetzner)"
    echo "  status  - Check service status"
    echo ""
    echo "Deployment order:"
    echo "  1. On AR:      ./deploy.sh certs"
    echo "  2. On AR:      ./deploy.sh server"
    echo "  3. Copy ca.crt to Hetzner"
    echo "  4. On Hetzner: ./deploy.sh client"
    ;;
esac
