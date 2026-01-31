#!/bin/bash
# Setup Tailscale for ClaudeBot Dashboard
#
# This script:
# 1. Installs Tailscale if not present
# 2. Authenticates with Tailscale
# 3. Configures HTTPS proxy for the dashboard
# 4. Displays the access URL
#
# Usage: ./scripts/setup-tailscale.sh [PORT]
#   PORT: Dashboard port (default: 8080)

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
DASHBOARD_PORT="${1:-8080}"

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if running as root for certain operations
check_sudo() {
    if [[ $EUID -ne 0 ]]; then
        if ! command -v sudo &> /dev/null; then
            log_error "This script requires sudo. Please install sudo or run as root."
            exit 1
        fi
        SUDO="sudo"
    else
        SUDO=""
    fi
}

# Check if Tailscale is installed
check_tailscale_installed() {
    if command -v tailscale &> /dev/null; then
        log_success "Tailscale is installed"
        return 0
    else
        return 1
    fi
}

# Install Tailscale
install_tailscale() {
    log_info "Installing Tailscale..."

    # Detect OS
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        OS=$ID
    else
        log_error "Cannot detect OS. Please install Tailscale manually."
        echo "Visit: https://tailscale.com/download"
        exit 1
    fi

    case "$OS" in
        ubuntu|debian|raspbian)
            # Add Tailscale repository
            curl -fsSL https://pkgs.tailscale.com/stable/$OS/$VERSION_CODENAME.noarmor.gpg | $SUDO tee /usr/share/keyrings/tailscale-archive-keyring.gpg >/dev/null
            curl -fsSL https://pkgs.tailscale.com/stable/$OS/$VERSION_CODENAME.tailscale-keyring.list | $SUDO tee /etc/apt/sources.list.d/tailscale.list
            $SUDO apt-get update
            $SUDO apt-get install -y tailscale
            ;;
        fedora|centos|rhel)
            $SUDO dnf config-manager --add-repo https://pkgs.tailscale.com/stable/fedora/tailscale.repo
            $SUDO dnf install -y tailscale
            ;;
        arch|manjaro)
            $SUDO pacman -S --noconfirm tailscale
            ;;
        *)
            # Fallback to curl installer
            log_warn "Unknown OS '$OS', using generic installer..."
            curl -fsSL https://tailscale.com/install.sh | sh
            ;;
    esac

    log_success "Tailscale installed"
}

# Start Tailscale daemon
start_tailscaled() {
    if ! systemctl is-active --quiet tailscaled; then
        log_info "Starting Tailscale daemon..."
        $SUDO systemctl enable --now tailscaled
        sleep 2
    fi
    log_success "Tailscale daemon is running"
}

# Check if authenticated
check_authenticated() {
    if tailscale status &> /dev/null; then
        return 0
    else
        return 1
    fi
}

# Authenticate with Tailscale
authenticate() {
    log_info "Authenticating with Tailscale..."
    echo ""
    echo "A browser window will open for authentication."
    echo "If it doesn't, follow the URL shown below."
    echo ""

    $SUDO tailscale up

    log_success "Authenticated with Tailscale"
}

# Configure Tailscale serve
configure_serve() {
    local port=$1

    log_info "Configuring HTTPS proxy for port $port..."

    # Check if already serving
    if tailscale serve status 2>/dev/null | grep -q "$port"; then
        log_warn "Port $port is already being served"
    else
        # Configure serve with HTTPS
        tailscale serve --bg --https=443 "$port"
        log_success "HTTPS proxy configured"
    fi
}

# Get Tailscale hostname and URL
get_tailscale_url() {
    local hostname
    hostname=$(tailscale status --json 2>/dev/null | jq -r '.Self.DNSName' | sed 's/\.$//')

    if [[ -n "$hostname" && "$hostname" != "null" ]]; then
        echo "https://$hostname"
    else
        log_error "Could not determine Tailscale hostname"
        return 1
    fi
}

# Get Tailscale IP
get_tailscale_ip() {
    tailscale ip -4 2>/dev/null || echo "unknown"
}

# Main
main() {
    echo ""
    echo "=========================================="
    echo "  ClaudeBot Tailscale Setup"
    echo "=========================================="
    echo ""

    check_sudo

    # Step 1: Install if needed
    if ! check_tailscale_installed; then
        install_tailscale
    fi

    # Step 2: Start daemon
    start_tailscaled

    # Step 3: Authenticate if needed
    if ! check_authenticated; then
        authenticate
    else
        log_success "Already authenticated with Tailscale"
    fi

    # Step 4: Configure serve
    configure_serve "$DASHBOARD_PORT"

    # Step 5: Display result
    echo ""
    echo "=========================================="
    echo "  Setup Complete!"
    echo "=========================================="
    echo ""

    local url
    url=$(get_tailscale_url)
    local ip
    ip=$(get_tailscale_ip)

    echo -e "Tailscale IP:  ${GREEN}$ip${NC}"
    echo -e "Dashboard URL: ${GREEN}$url${NC}"
    echo ""
    echo "You can access the dashboard from any device on your Tailnet."
    echo ""
    echo "Useful commands:"
    echo "  tailscale status       - Show connected devices"
    echo "  tailscale serve status - Show serve configuration"
    echo "  tailscale serve off    - Disable HTTPS proxy"
    echo ""
}

# Run main
main "$@"
