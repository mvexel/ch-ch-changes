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
DEPLOY_DIR="/opt/chch"
DATA_DIR="/data/parquet"
SERVICE_USER="chch"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo -e "\n${YELLOW}[1/8] Creating system user...${NC}"
if id "$SERVICE_USER" &>/dev/null; then
    echo "User $SERVICE_USER already exists"
else
    useradd -r -s /bin/false -d "$DEPLOY_DIR" -c "OSM Changeset Server" "$SERVICE_USER"
    echo -e "${GREEN}✓ Created user $SERVICE_USER${NC}"
fi

echo -e "\n${YELLOW}[2/8] Creating directory structure...${NC}"
mkdir -p "$DEPLOY_DIR"/{bin,releases,data}
mkdir -p "$DATA_DIR"
mkdir -p /var/lib/chch
echo -e "${GREEN}✓ Created directories${NC}"

ENV_FILE="$DEPLOY_DIR/.env"
if [ ! -f "$ENV_FILE" ]; then
    if [ -f "$REPO_DIR/.env" ]; then
        install -m 640 "$REPO_DIR/.env" "$ENV_FILE"
    else
        cat > "$ENV_FILE" <<'EOF'
# chch-server environment
OSM_OAUTH_CLIENT_ID=
OSM_OAUTH_CLIENT_SECRET=
OSM_OAUTH_REDIRECT_URL=
AUTH_KEY_SECRET=
AUTH_DB_PATH=/var/lib/chch/auth.db
AUTH_DEFAULT_DAILY_LIMIT=0
AUTH_DEFAULT_MONTHLY_LIMIT=0
REQUIRE_AUTH=true
DATA_PATH=/opt/chch/data

# Replication worker
POLL_INTERVAL_SECONDS=60
CHANGESET_REPLICATION_URL=https://planet.openstreetmap.org/replication/changesets/
HOME=/var/lib/chch
ENABLE_COMPACTION=true
COMPACTION_AGE_HOURS=24
EOF
        chmod 640 "$ENV_FILE"
    fi
    echo -e "${GREEN}✓ Created env file at $ENV_FILE${NC}"
else
    echo "Env file already exists at $ENV_FILE"
fi

echo -e "\n${YELLOW}[3/8] Setting permissions...${NC}"
chown -R "$SERVICE_USER:$SERVICE_USER" "$DEPLOY_DIR" "$DATA_DIR" /var/lib/chch
chmod 640 "$ENV_FILE"
echo -e "${GREEN}✓ Set ownership to $SERVICE_USER${NC}"

echo -e "\n${YELLOW}[4/8] Installing systemd services...${NC}"
cp "$REPO_DIR/systemd/chch-server.service" /etc/systemd/system/
cp "$REPO_DIR/systemd/chch-worker.service" /etc/systemd/system/
systemctl daemon-reload
echo -e "${GREEN}✓ Installed systemd services${NC}"

echo -e "\n${YELLOW}[5/8] Making scripts executable...${NC}"
chmod +x "$REPO_DIR/scripts"/*.sh
echo -e "${GREEN}✓ Scripts are executable${NC}"

echo -e "\n${YELLOW}[6/8] Checking Python installation...${NC}"
if ! command -v python3 &> /dev/null; then
    echo -e "${RED}Error: Python 3 not found${NC}"
    echo "Please install Python 3.11 or higher"
    exit 1
fi

PYTHON_VERSION=$(python3 --version | awk '{print $2}')
echo "Python version: $PYTHON_VERSION"

# Check Python version is 3.11+
MAJOR=$(echo "$PYTHON_VERSION" | cut -d. -f1)
MINOR=$(echo "$PYTHON_VERSION" | cut -d. -f2)
if [ "$MAJOR" -lt 3 ] || ([ "$MAJOR" -eq 3 ] && [ "$MINOR" -lt 11 ]); then
    echo -e "${RED}Error: Python 3.11+ required (found $PYTHON_VERSION)${NC}"
    exit 1
fi

# Install Rust for the converter tool (optional but recommended)
echo -e "\n${YELLOW}Installing Rust for converter (optional)...${NC}"
if command -v cargo &> /dev/null; then
    echo "Rust already installed: $(cargo --version)"
else
    echo "Installing Rust for planet dump converter..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    echo -e "${GREEN}✓ Installed Rust${NC}"
fi

echo -e "\n${YELLOW}[7/8] Building and deploying initial version...${NC}"
cd "$REPO_DIR"

# Run deploy as the user who invoked sudo
ACTUAL_USER=$(logname 2>/dev/null || echo "$SUDO_USER")
if [ -n "$ACTUAL_USER" ]; then
    sudo -u "$ACTUAL_USER" "$REPO_DIR/scripts/deploy.sh" "initial"
else
    echo -e "${YELLOW}Warning: Could not determine user, running deploy as root${NC}"
    "$REPO_DIR/scripts/deploy.sh" "initial"
fi

echo -e "\n${YELLOW}[8/8] Configuring services...${NC}"
# Stop services that were started by deploy.sh - they need data first
systemctl stop chch-server chch-worker 2>/dev/null || true
# Enable to start on boot (but don't start yet)
systemctl enable chch-server
systemctl enable chch-worker
echo -e "${GREEN}✓ Services configured (not started - waiting for data)${NC}"

# Summary
echo -e "\n${GREEN}=== Setup Complete ===${NC}"
echo ""
echo -e "${YELLOW}⚠ IMPORTANT: Services are NOT running yet - they need data first!${NC}"
echo ""
echo "Next steps:"
echo "  ${YELLOW}1. Load initial data (REQUIRED before starting services):${NC}"
echo "     $REPO_DIR/scripts/init-data.sh"
echo ""
echo "     Alternative options:"
echo "     # Use existing file:"
echo "     $REPO_DIR/scripts/init-data.sh --input /path/to/changesets.osm.bz2"
echo "     # Or copy existing parquet files:"
echo "     sudo cp -r /path/to/data/* $DEPLOY_DIR/data/"
echo ""
echo "  ${YELLOW}2. Start services (ONLY after data is loaded):${NC}"
echo "     sudo systemctl start chch-server chch-worker"
echo ""
echo "  3. Verify services are running:"
echo "     sudo systemctl status chch-server chch-worker"
echo ""
echo "  4. Check logs:"
echo "     journalctl -u chch-server -f"
echo "     journalctl -u chch-worker -f"
echo ""
echo "  5. Test the API:"
echo "     curl http://localhost:3000/health"
echo "     curl 'http://localhost:3000/changesets?bbox=-180,-90,180,90&limit=10'"
echo ""
echo "Python environment:"
echo "  Virtual env: $DEPLOY_DIR/venv"
echo "  Activate:    source $DEPLOY_DIR/venv/bin/activate"
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
