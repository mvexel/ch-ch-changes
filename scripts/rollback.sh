#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

DEPLOY_DIR="/opt/changeset-server"

# Check if version argument is provided
if [ $# -eq 0 ]; then
    echo -e "${RED}Error: No version specified${NC}"
    echo ""
    echo "Usage: $0 <version>"
    echo ""
    echo "Available versions:"
    ls -1 "$DEPLOY_DIR/releases/" | grep -v current || echo "  (none found)"
    exit 1
fi

VERSION="$1"
RELEASE_DIR="$DEPLOY_DIR/releases/$VERSION"

# Check if version exists
if [ ! -d "$RELEASE_DIR" ]; then
    echo -e "${RED}Error: Version $VERSION not found${NC}"
    echo ""
    echo "Available versions:"
    ls -1 "$DEPLOY_DIR/releases/" | grep -v current
    exit 1
fi

# Get current version for confirmation
CURRENT=$(readlink "$DEPLOY_DIR/releases/current" | xargs basename)

echo -e "${YELLOW}=== Rolling back changeset-server ===${NC}"
echo "Current version: $CURRENT"
echo "Target version:  $VERSION"
echo ""
read -p "Continue with rollback? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Rollback cancelled"
    exit 0
fi

# Update symlinks
echo -e "\n${YELLOW}[1/4] Updating symlinks...${NC}"
ln -sfn "$RELEASE_DIR" "$DEPLOY_DIR/releases/current"
ln -sfn "$DEPLOY_DIR/releases/current/changeset-server" "$DEPLOY_DIR/bin/changeset-server"
ln -sfn "$DEPLOY_DIR/releases/current/changesets-to-parquet" "$DEPLOY_DIR/bin/changesets-to-parquet"
ln -sfn "$DEPLOY_DIR/releases/current/changeset-replication-worker" "$DEPLOY_DIR/bin/changeset-replication-worker"

# Update /usr/local/bin symlinks if directory exists
if [ -d "/usr/local/bin" ]; then
    echo "Updating /usr/local/bin symlinks..."
    sudo ln -sf "$DEPLOY_DIR/bin/changeset-server" /usr/local/bin/changeset-server
    sudo ln -sf "$DEPLOY_DIR/bin/changesets-to-parquet" /usr/local/bin/changesets-to-parquet
    sudo ln -sf "$DEPLOY_DIR/bin/changeset-replication-worker" /usr/local/bin/changeset-replication-worker
fi

# Restart service
echo -e "\n${YELLOW}[2/4] Stopping service...${NC}"
sudo systemctl stop changeset-server

echo -e "\n${YELLOW}[3/4] Starting service...${NC}"
sudo systemctl start changeset-server

# Health check
echo -e "\n${YELLOW}[4/4] Running health checks...${NC}"
sleep 3

if systemctl is-active --quiet changeset-server; then
    echo -e "${GREEN}✓ Service is running${NC}"
else
    echo -e "${RED}✗ Service failed to start${NC}"
    echo "Recent logs:"
    journalctl -u changeset-server -n 20 --no-pager
    exit 1
fi

if curl -sf http://localhost:3000/health > /dev/null 2>&1; then
    echo -e "${GREEN}✓ Health check passed${NC}"
else
    echo -e "${YELLOW}⚠ Health check failed (service may still be starting)${NC}"
fi

echo -e "\n${GREEN}=== Rollback Complete ===${NC}"
echo "Rolled back from $CURRENT to $VERSION"
systemctl status changeset-server --no-pager -l
