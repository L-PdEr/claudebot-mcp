# ClaudeBot Remote Access Guide

> Secure remote access to your ClaudeBot dashboard from anywhere.

## Overview

By default, the ClaudeBot dashboard binds to `127.0.0.1` (localhost only) for security. This guide covers how to securely access the dashboard from other devices.

**Security Principle:** Never expose the dashboard directly to the internet. Always use a secure tunnel.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Security Options Comparison                       │
├─────────────────────┬────────────┬────────────┬─────────────────────┤
│       Option        │ Complexity │  Security  │      Best For       │
├─────────────────────┼────────────┼────────────┼─────────────────────┤
│ Localhost Only      │ ⭐         │ ⭐⭐⭐⭐⭐ │ Single machine      │
│ Tailscale (rec.)    │ ⭐⭐       │ ⭐⭐⭐⭐⭐ │ Multi-device        │
│ Cloudflare Tunnel   │ ⭐⭐⭐     │ ⭐⭐⭐⭐   │ Team/public access  │
│ WireGuard Manual    │ ⭐⭐⭐⭐   │ ⭐⭐⭐⭐⭐ │ Self-hosted VPN     │
│ Direct Port Forward │ ⭐         │ ⭐         │ ⛔ NEVER DO THIS    │
└─────────────────────┴────────────┴────────────┴─────────────────────┘
```

---

## Option 1: Tailscale (Recommended)

[Tailscale](https://tailscale.com) creates a secure mesh VPN between your devices using WireGuard. It's free for personal use (up to 100 devices).

### Why Tailscale?

- **Zero configuration**: No firewall rules, port forwarding, or DNS setup
- **Works everywhere**: Through NAT, CGNAT, firewalls
- **End-to-end encrypted**: WireGuard protocol
- **Auto HTTPS**: Free TLS certificates via `tailscale serve`
- **MagicDNS**: Access via `hostname.tailnet-name.ts.net`

### Quick Setup

```bash
# Run the setup script
./scripts/setup-tailscale.sh
```

Or manually:

```bash
# 1. Install Tailscale
curl -fsSL https://tailscale.com/install.sh | sh

# 2. Authenticate
sudo tailscale up

# 3. Expose dashboard with HTTPS
tailscale serve --bg --https=443 8080

# 4. Get your URL
tailscale status --json | jq -r '.Self.DNSName' | sed 's/\.$//'
# Output: your-hostname.tailnet-name.ts.net
```

### Access the Dashboard

From any device on your Tailnet:
```
https://your-hostname.tailnet-name.ts.net
```

### Security Notes

- Traffic is encrypted end-to-end (WireGuard)
- No ports exposed to the public internet
- Access is limited to devices in your Tailnet
- You can use Tailscale ACLs for fine-grained access control

### Tailscale ACL Example

To restrict dashboard access to specific users:

```json
{
  "acls": [
    {
      "action": "accept",
      "src": ["tag:admin"],
      "dst": ["tag:claudebot:443"]
    }
  ],
  "tagOwners": {
    "tag:admin": ["your-email@example.com"],
    "tag:claudebot": ["your-email@example.com"]
  }
}
```

---

## Option 2: Cloudflare Tunnel

[Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/) (formerly Argo Tunnel) creates a secure outbound connection to Cloudflare's network.

### Setup

```bash
# 1. Install cloudflared
# Debian/Ubuntu
curl -L https://pkg.cloudflare.com/cloudflared-stable-linux-amd64.deb -o cloudflared.deb
sudo dpkg -i cloudflared.deb

# 2. Authenticate
cloudflared tunnel login

# 3. Create tunnel
cloudflared tunnel create claudebot

# 4. Configure tunnel
cat > ~/.cloudflared/config.yml << EOF
tunnel: claudebot
credentials-file: /root/.cloudflared/<TUNNEL_ID>.json

ingress:
  - hostname: claudebot.yourdomain.com
    service: http://localhost:8080
  - service: http_status:404
EOF

# 5. Route DNS
cloudflared tunnel route dns claudebot claudebot.yourdomain.com

# 6. Run tunnel
cloudflared tunnel run claudebot
```

### Security Notes

- Requires a domain you control
- Cloudflare sees your traffic (not end-to-end encrypted)
- You can add Cloudflare Access for authentication
- Good for team access with SSO integration

---

## Option 3: WireGuard (Manual)

For self-hosted VPN with full control.

### Server Setup

```bash
# 1. Install WireGuard
sudo apt install wireguard

# 2. Generate keys
wg genkey | tee privatekey | wg pubkey > publickey

# 3. Configure server
sudo cat > /etc/wireguard/wg0.conf << EOF
[Interface]
PrivateKey = $(cat privatekey)
Address = 10.0.0.1/24
ListenPort = 51820

[Peer]
PublicKey = <CLIENT_PUBLIC_KEY>
AllowedIPs = 10.0.0.2/32
EOF

# 4. Start WireGuard
sudo wg-quick up wg0
sudo systemctl enable wg-quick@wg0
```

### Client Setup

```bash
# Generate client keys
wg genkey | tee client_privatekey | wg pubkey > client_publickey

# Configure client
cat > wg0.conf << EOF
[Interface]
PrivateKey = $(cat client_privatekey)
Address = 10.0.0.2/24
DNS = 1.1.1.1

[Peer]
PublicKey = <SERVER_PUBLIC_KEY>
Endpoint = your-server-ip:51820
AllowedIPs = 10.0.0.0/24
PersistentKeepalive = 25
EOF
```

### Access Dashboard

```
http://10.0.0.1:8080
```

---

## Option 4: SSH Port Forwarding

For quick, temporary access.

```bash
# From your local machine
ssh -L 8080:localhost:8080 user@server

# Then access
http://localhost:8080
```

This forwards your local port 8080 to the server's localhost:8080.

---

## Dashboard Configuration

### Enable LAN Access

To bind to all interfaces (required for remote access):

```bash
# Environment variable
export DASHBOARD_HOST=0.0.0.0

# Or in config
[dashboard]
host = "0.0.0.0"
port = 8080
require_auth = true  # Automatically enabled for non-localhost
```

### Check Tailscale Status

The dashboard API includes a Tailscale status endpoint:

```bash
curl http://localhost:8080/api/network/tailscale
```

Response:
```json
{
  "installed": true,
  "running": true,
  "hostname": "claudebot",
  "tailnet": "your-tailnet.ts.net",
  "ip": "100.x.y.z",
  "url": "https://claudebot.your-tailnet.ts.net"
}
```

---

## Troubleshooting

### Tailscale not connecting

```bash
# Check status
tailscale status

# Check logs
journalctl -u tailscaled -f

# Re-authenticate
sudo tailscale up --reset
```

### Dashboard not accessible

```bash
# Verify dashboard is running
curl http://localhost:8080/api/health

# Check binding address
ss -tlnp | grep 8080

# Check firewall (shouldn't need changes for Tailscale)
sudo ufw status
```

### Certificate errors

If using `tailscale serve`, certificates are automatic. For other methods:

```bash
# Check certificate
openssl s_client -connect your-hostname:443 -servername your-hostname
```

---

## Security Best Practices

1. **Never expose directly**: Always use a secure tunnel
2. **Enable authentication**: Required automatically for non-localhost
3. **Use MFA**: Enable TOTP when available (D3.3)
4. **Review access logs**: Check `/api/logs` for suspicious activity
5. **Keep software updated**: Tailscale, WireGuard, and ClaudeBot
6. **Use ACLs**: Restrict access to specific users/devices
7. **Monitor connections**: Use `tailscale status` or WireGuard logs

---

## See Also

- [Tailscale Documentation](https://tailscale.com/kb/)
- [Tailscale Serve](https://tailscale.com/kb/1242/tailscale-serve)
- [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/)
- [WireGuard](https://www.wireguard.com/)
