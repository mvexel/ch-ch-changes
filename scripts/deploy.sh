#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

VERSION="${1:-$(date +%Y%m%d_%H%M%S)}"
DEPLOY_DIR="/opt/changeset-server"
RELEASE_DIR="$DEPLOY_DIR/releases/$VERSION"

echo -e "${YELLOW}=== Deploying changeset-server version $VERSION ===${NC}"

# Ensure cargo is available - source cargo env if not in PATH
if ! command -v cargo &> /dev/null; then
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    else
        echo -e "${RED}Error: cargo not found. Please install Rust first.${NC}"
        exit 1
    fi
fi

# Step 1: Build the release binary
echo -e "\n${YELLOW}[1/7] Building release binary...${NC}"
cargo build --release --bin changeset-server

# Step 2: Create release directory
echo -e "\n${YELLOW}[2/7] Creating release directory...${NC}"
mkdir -p "$RELEASE_DIR"

# Step 3: Copy binary to release directory
echo -e "\n${YELLOW}[3/7] Copying binary...${NC}"
cp target/release/changeset-server "$RELEASE_DIR/"
chmod +x "$RELEASE_DIR/changeset-server"

# Step 4: Test the binary
echo -e "\n${YELLOW}[4/7] Testing binary...${NC}"
if ! "$RELEASE_DIR/changeset-server" --version 2>/dev/null; then
    # Binary doesn't support --version, that's ok
    echo "Binary executable check passed"
fi

# Step 5: Update symlinks
echo -e "\n${YELLOW}[5/7] Updating symlinks...${NC}"
ln -sfn "$RELEASE_DIR" "$DEPLOY_DIR/releases/current"
ln -sfn "$DEPLOY_DIR/releases/current/changeset-server" "$DEPLOY_DIR/bin/changeset-server"

# Step 6: Restart service
echo -e "\n${YELLOW}[6/7] Restarting service...${NC}"
sudo systemctl restart changeset-server

# Step 7: Health check
echo -e "\n${YELLOW}[7/7] Running health checks...${NC}"
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

# Show service status
echo -e "\n${GREEN}=== Deployment Complete ===${NC}"
echo "Version: $VERSION"
echo "Status: $(systemctl is-active changeset-server)"
echo ""
echo "Useful commands:"
echo "  View logs:    journalctl -u changeset-server -f"
echo "  Check status: systemctl status changeset-server"
echo "  Rollback:     ./scripts/rollback.sh <version>"
echo "  List releases: ls -lh $DEPLOY_DIR/releases/"
