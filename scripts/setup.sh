#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}=== OSM Changeset Server Setup ===${NC}"
echo "This script will set up the changeset server for native deployment"
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo -e "${RED}Error: This script must be run as root${NC}"
    echo "Please run: sudo $0"
    exit 1
fi

# Configuration
DEPLOY_DIR="/opt/changeset-server"
DATA_DIR="/data/parquet"
SERVICE_USER="changeset"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo -e "\n${YELLOW}[1/8] Creating system user...${NC}"
if id "$SERVICE_USER" &>/dev/null; then
    echo "User $SERVICE_USER already exists"
else
    useradd -r -s /bin/false -d "$DEPLOY_DIR" -c "OSM Changeset Server" "$SERVICE_USER"
    echo -e "${GREEN}✓ Created user $SERVICE_USER${NC}"
fi

echo -e "\n${YELLOW}[2/8] Creating directory structure...${NC}"
mkdir -p "$DEPLOY_DIR"/{bin,releases,shared}
mkdir -p "$DATA_DIR"
mkdir -p /var/lib/changeset-server
echo -e "${GREEN}✓ Created directories${NC}"

echo -e "\n${YELLOW}[3/8] Setting permissions...${NC}"
chown -R "$SERVICE_USER:$SERVICE_USER" "$DEPLOY_DIR" "$DATA_DIR" /var/lib/changeset-server
echo -e "${GREEN}✓ Set ownership to $SERVICE_USER${NC}"

echo -e "\n${YELLOW}[4/8] Installing systemd service...${NC}"
cp "$REPO_DIR/systemd/changeset-server.service" /etc/systemd/system/
systemctl daemon-reload
echo -e "${GREEN}✓ Installed systemd service${NC}"

echo -e "\n${YELLOW}[5/8] Making scripts executable...${NC}"
chmod +x "$REPO_DIR/scripts"/*.sh
echo -e "${GREEN}✓ Scripts are executable${NC}"

echo -e "\n${YELLOW}[6/8] Installing Rust (if needed)...${NC}"
if command -v cargo &> /dev/null; then
    echo "Rust already installed: $(cargo --version)"
else
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo -e "${GREEN}✓ Installed Rust${NC}"
fi

echo -e "\n${YELLOW}[7/8] Building and deploying initial version...${NC}"
cd "$REPO_DIR"
"$REPO_DIR/scripts/deploy.sh" "initial"

echo -e "\n${YELLOW}[8/8] Enabling service...${NC}"
systemctl enable changeset-server
echo -e "${GREEN}✓ Service enabled to start on boot${NC}"

# Summary
echo -e "\n${GREEN}=== Setup Complete ===${NC}"
echo ""
echo "Service status:"
systemctl status changeset-server --no-pager -l || true
echo ""
echo "Next steps:"
echo "  1. Copy your parquet files to: $DATA_DIR"
echo "  2. Restart the service: systemctl restart changeset-server"
echo "  3. Check logs: journalctl -u changeset-server -f"
echo "  4. Test the API: curl http://localhost:3000/health"
echo ""
echo "Optional - Set up monitoring:"
echo "  # Add to crontab for health checks every 5 minutes"
echo "  */5 * * * * $REPO_DIR/scripts/healthcheck.sh"
echo ""
echo "Optional - Set up reverse proxy (Caddy example):"
echo "  apt install -y debian-keyring debian-archive-keyring apt-transport-https curl"
echo "  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg"
echo "  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list"
echo "  apt update && apt install caddy"
echo "  # Then configure Caddy for HTTPS/domain"
