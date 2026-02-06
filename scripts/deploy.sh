#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

VERSION="${1:-$(date +%Y%m%d_%H%M%S)}"
DEPLOY_DIR="/opt/chch"
RELEASE_DIR="$DEPLOY_DIR/releases/$VERSION"

echo -e "${YELLOW}=== Deploying ch-ch-changes version $VERSION ===${NC}"

# Ensure cargo is available - source cargo env if not in PATH
if ! command -v cargo &> /dev/null; then
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    else
        echo -e "${RED}Error: cargo not found. Please install Rust first.${NC}"
        exit 1
    fi
fi

# Step 1: Build the release binaries
echo -e "\n${YELLOW}[1/8] Building release binaries...${NC}"
cargo build --release --workspace

# Step 2: Create release directory
echo -e "\n${YELLOW}[2/8] Creating release directory...${NC}"
mkdir -p "$RELEASE_DIR"

# Step 3: Copy binary to release directory
echo -e "\n${YELLOW}[3/8] Copying binaries...${NC}"
cp target/release/chch-server "$RELEASE_DIR/"
cp target/release/chch-convert "$RELEASE_DIR/"
cp target/release/chch-worker "$RELEASE_DIR/"
chmod +x "$RELEASE_DIR/chch-server"
chmod +x "$RELEASE_DIR/chch-convert"
chmod +x "$RELEASE_DIR/chch-worker"

# Step 4: Test the binary
echo -e "\n${YELLOW}[4/8] Testing binary...${NC}"
if [ ! -x "$RELEASE_DIR/chch-server" ]; then
    echo -e "${RED}Error: Binary is not executable${NC}"
    exit 1
fi
# Just verify it's a valid ELF binary
if ! file "$RELEASE_DIR/chch-server" | grep -q "ELF.*executable"; then
    echo -e "${RED}Error: Not a valid executable${NC}"
    exit 1
fi
echo "Binary check passed"

echo -e "\n${YELLOW}[5/8] Stopping service...${NC}"
sudo systemctl stop chch-server

# Step 5: Update symlinks
echo -e "\n${YELLOW}[6/8] Updating symlinks...${NC}"
ln -sfn "$RELEASE_DIR" "$DEPLOY_DIR/releases/current"
ln -sfn "$DEPLOY_DIR/releases/current/chch-server" "$DEPLOY_DIR/bin/chch-server"
ln -sfn "$DEPLOY_DIR/releases/current/chch-convert" "$DEPLOY_DIR/bin/chch-convert"
ln -sfn "$DEPLOY_DIR/releases/current/chch-worker" "$DEPLOY_DIR/bin/chch-worker"

# Ensure binaries are in PATH
if [ -d "/usr/local/bin" ]; then
    echo "Linking binaries to /usr/local/bin..."
    sudo ln -sf "$DEPLOY_DIR/bin/chch-server" /usr/local/bin/chch-server
    sudo ln -sf "$DEPLOY_DIR/bin/chch-convert" /usr/local/bin/chch-convert
    sudo ln -sf "$DEPLOY_DIR/bin/chch-worker" /usr/local/bin/chch-worker
fi

# Step 6: Restart service
echo -e "\n${YELLOW}[7/8] Restarting service...${NC}"
sudo systemctl start chch-server

# Step 7: Health check
echo -e "\n${YELLOW}[8/8] Running health checks...${NC}"
sleep 3

if systemctl is-active --quiet chch-server; then
    echo -e "${GREEN}✓ Service is running${NC}"
else
    echo -e "${RED}✗ Service failed to start${NC}"
    echo "Recent logs:"
    journalctl -u chch-server -n 20 --no-pager
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
echo "Status: $(systemctl is-active chch-server)"
echo ""
echo "Useful commands:"
echo "  View logs:    journalctl -u chch-server -f"
echo "  Check status: systemctl status chch-server"
echo "  Rollback:     ./scripts/rollback.sh <version>"
echo "  List releases: ls -lh $DEPLOY_DIR/releases/"
