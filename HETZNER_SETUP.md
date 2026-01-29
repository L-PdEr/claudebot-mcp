# ClaudeBot MCP - Hetzner Server Setup

> **Full prompt for Claude Code to set up claudebot-mcp on a fresh Hetzner VPS**

---

## Pre-Setup Checklist

Before running this prompt on your Hetzner server:

1. Fresh Ubuntu 22.04/24.04 VPS (CX21 or higher recommended)
2. SSH access as root or sudo user
3. Your `ANTHROPIC_API_KEY` ready

---

## Claude Code Setup Prompt

Copy and paste this entire prompt into Claude Code on your Hetzner server:

```
I need you to set up claudebot-mcp on this fresh Hetzner VPS. Please do the following:

## 1. System Cleanup & Preparation

First, clean up any old firewall rules and prepare the system:

```bash
# Reset UFW to defaults
sudo ufw --force reset
sudo ufw default deny incoming
sudo ufw default allow outgoing

# Allow SSH (important - don't lock yourself out!)
sudo ufw allow 22/tcp

# Enable UFW
sudo ufw --force enable
sudo ufw status verbose
```

Clean up old services if they exist:
```bash
# Stop and disable old clawdbot services
sudo systemctl stop clawdbot-bridge.service 2>/dev/null || true
sudo systemctl stop clawdbot-telegram.service 2>/dev/null || true
sudo systemctl stop clawdbot-server.service 2>/dev/null || true
sudo systemctl disable clawdbot-bridge.service 2>/dev/null || true
sudo systemctl disable clawdbot-telegram.service 2>/dev/null || true
sudo systemctl disable clawdbot-server.service 2>/dev/null || true

# Remove old service files
sudo rm -f /etc/systemd/system/clawdbot-*.service
sudo systemctl daemon-reload

# Clean up old binaries
sudo rm -f /usr/local/bin/clawdbot-*
```

## 2. Install Dependencies

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install build essentials
sudo apt install -y build-essential pkg-config libssl-dev curl git

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env

# Verify Rust installation
rustc --version
cargo --version
```

## 3. Clone and Build

```bash
# Create app directory
mkdir -p ~/apps
cd ~/apps

# Clone the repository (or copy files)
git clone https://github.com/L-PdEr/velofi.git quantum-nexus-trading
cd quantum-nexus-trading/claudebot-mcp

# Build release binary
cargo build --release

# Verify build
./target/release/claudebot-mcp --help 2>&1 || echo "Binary built successfully"
```

## 4. Configure

```bash
# Create config
cp .env.example .env

# Set your API key (replace with your actual key)
echo "ANTHROPIC_API_KEY=sk-ant-api03-YOUR-KEY-HERE" > .env
echo "CLAUDEBOT_MODEL=opus" >> .env
echo "CLAUDEBOT_DB_PATH=/var/lib/claudebot-mcp/data.db" >> .env

# Create data directory
sudo mkdir -p /var/lib/claudebot-mcp
sudo chown $USER:$USER /var/lib/claudebot-mcp
```

## 5. Install System-wide

```bash
# Copy binary to /usr/local/bin
sudo cp target/release/claudebot-mcp /usr/local/bin/
sudo chmod +x /usr/local/bin/claudebot-mcp

# Copy config
sudo mkdir -p /etc/claudebot-mcp
sudo cp .env /etc/claudebot-mcp/.env
sudo chmod 600 /etc/claudebot-mcp/.env
```

## 6. Create Systemd Service (Optional - for daemon mode)

Create `/etc/systemd/system/claudebot-mcp.service`:

```ini
[Unit]
Description=ClaudeBot MCP Server
After=network.target

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/etc/claudebot-mcp
EnvironmentFile=/etc/claudebot-mcp/.env
ExecStart=/usr/local/bin/claudebot-mcp
Restart=on-failure
RestartSec=5

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/var/lib/claudebot-mcp

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable claudebot-mcp
sudo systemctl start claudebot-mcp
sudo systemctl status claudebot-mcp
```

## 7. Configure Claude Code on Hetzner

Add to `~/.config/claude-code/config.json`:

```json
{
  "mcpServers": {
    "claudebot": {
      "command": "/usr/local/bin/claudebot-mcp",
      "env": {
        "ANTHROPIC_API_KEY": "${ANTHROPIC_API_KEY}"
      }
    }
  }
}
```

## 8. Verify Installation

```bash
# Test MCP server manually
echo '{"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{}},"id":1}' | /usr/local/bin/claudebot-mcp

# Check Claude Code sees the tools
claude --version
```

## 9. Optional: Install Ollama for Local LLM Routing

```bash
# Install Ollama
curl -fsSL https://ollama.com/install.sh | sh

# Pull Llama model
ollama pull llama3.2:3b

# Create systemd service for Ollama
sudo systemctl enable ollama
sudo systemctl start ollama

# Add to claudebot config
echo "CLAUDEBOT_OLLAMA_URL=http://localhost:11434" | sudo tee -a /etc/claudebot-mcp/.env
```

## 10. Firewall Rules (if needed)

For MCP over stdio, no additional ports needed. If you add HTTP/gRPC later:

```bash
# Only if exposing API endpoints
sudo ufw allow 8080/tcp  # HTTP API
sudo ufw allow 50051/tcp # gRPC
sudo ufw status
```

Please execute these steps and report any errors.
```

---

## Quick One-Liner Setup

For an already-configured server, just run:

```bash
cd ~/apps/quantum-nexus-trading/claudebot-mcp && \
git pull && \
cargo build --release && \
sudo cp target/release/claudebot-mcp /usr/local/bin/ && \
sudo systemctl restart claudebot-mcp
```

---

## Cleanup Old Installation

If you have an old clawdbot-grpc installation:

```bash
# Stop old services
sudo systemctl stop clawdbot-bridge clawdbot-telegram clawdbot-server 2>/dev/null

# Disable old services
sudo systemctl disable clawdbot-bridge clawdbot-telegram clawdbot-server 2>/dev/null

# Remove old service files
sudo rm -f /etc/systemd/system/clawdbot-*.service
sudo systemctl daemon-reload

# Remove old binaries
sudo rm -f /usr/local/bin/clawdbot-bridge
sudo rm -f /usr/local/bin/clawdbot-server
sudo rm -f /usr/local/bin/clawdbot-telegram

# Remove old configs (backup first if needed)
sudo rm -rf /etc/clawdbot-grpc

# Clean firewall rules
sudo ufw delete allow 50051/tcp 2>/dev/null  # old gRPC port
sudo ufw status
```

---

## Server Specifications

**Minimum Requirements:**
- CPU: 2 vCPUs
- RAM: 4GB
- Storage: 20GB SSD
- OS: Ubuntu 22.04 LTS

**Recommended (with Ollama):**
- CPU: 4 vCPUs
- RAM: 8GB
- Storage: 40GB SSD
- OS: Ubuntu 24.04 LTS

**Hetzner Server Types:**
- CX21: 2 vCPU, 4GB RAM - Minimum
- CX31: 2 vCPU, 8GB RAM - Recommended
- CX41: 4 vCPU, 16GB RAM - With Ollama

---

## Monitoring

```bash
# Check service status
sudo systemctl status claudebot-mcp

# View logs
sudo journalctl -u claudebot-mcp -f

# Check resource usage
htop
```

---

## Security Notes

1. **API Key**: Never commit `.env` files with real keys
2. **Firewall**: Keep UFW enabled, only open necessary ports
3. **Updates**: Regularly update system and rebuild
4. **Backups**: Backup `/var/lib/claudebot-mcp/` for memory persistence
