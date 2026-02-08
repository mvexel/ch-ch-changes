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
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_SOURCE="$REPO_DIR/.env"
ENV_TARGET="$DEPLOY_DIR/.env"
VENV_DIR="$DEPLOY_DIR/venv"

echo -e "${YELLOW}=== Deploying ch-ch-changes version $VERSION ===${NC}"

# Step 1: Build the Python package
echo -e "\n${YELLOW}[1/9] Building Python package...${NC}"
cd "$REPO_DIR"
python3 -m pip install --upgrade build
python3 -m build

# Step 2: Create release directory
echo -e "\n${YELLOW}[2/9] Creating release directory...${NC}"
mkdir -p "$RELEASE_DIR"

# Step 3: Install env file (if missing)
echo -e "\n${YELLOW}[3/9] Installing env file (if missing)...${NC}"
if [ -f "$ENV_SOURCE" ]; then
    if [ ! -f "$ENV_TARGET" ]; then
        sudo install -m 640 "$ENV_SOURCE" "$ENV_TARGET"
        if id "chch" &>/dev/null; then
            sudo chown chch:chch "$ENV_TARGET"
        fi
        echo "Installed $ENV_TARGET"
    else
        echo "Env file already exists at $ENV_TARGET (leaving as-is)"
    fi
else
    echo "No repo .env found at $ENV_SOURCE (skipping)"
fi

# Step 4: Create/update virtual environment
echo -e "\n${YELLOW}[4/9] Setting up virtual environment...${NC}"
if [ ! -d "$VENV_DIR" ]; then
    python3 -m venv "$VENV_DIR"
fi
source "$VENV_DIR/bin/activate"
pip install --upgrade pip
pip install "$REPO_DIR/dist/"*.whl
echo -e "${GREEN}✓ Installed Python package${NC}"

# Step 5: Build Rust converter (for initial planet dump loads)
echo -e "\n${YELLOW}[5/9] Building Rust converter...${NC}"
if [ -d "$REPO_DIR/rust-converter" ]; then
    # Ensure cargo is available
    if ! command -v cargo &> /dev/null; then
        if [ -f "$HOME/.cargo/env" ]; then
            source "$HOME/.cargo/env"
        else
            echo -e "${YELLOW}Warning: cargo not found, skipping converter build${NC}"
        fi
    fi

    if command -v cargo &> /dev/null; then
        cd "$REPO_DIR/rust-converter"
        cargo build --release
        mkdir -p "$DEPLOY_DIR/bin"
        cp target/release/chch-convert "$DEPLOY_DIR/bin/"
        chmod +x "$DEPLOY_DIR/bin/chch-convert"
        echo -e "${GREEN}✓ Built Rust converter${NC}"
    fi
    cd "$REPO_DIR"
else
    echo -e "${YELLOW}No rust-converter directory found, skipping${NC}"
fi

echo -e "\n${YELLOW}[6/9] Stopping services...${NC}"
sudo systemctl stop chch-server chch-worker

# Step 7: Update release tracking
echo -e "\n${YELLOW}[7/9] Recording release...${NC}"
echo "$VERSION" > "$DEPLOY_DIR/releases/current_version"

# Ensure converter is accessible
if [ -f "$DEPLOY_DIR/bin/chch-convert" ] && [ -d "/usr/local/bin" ]; then
    echo "Linking converter to /usr/local/bin..."
    sudo ln -sf "$DEPLOY_DIR/bin/chch-convert" /usr/local/bin/chch-convert
fi

# Step 8: Restart services
echo -e "\n${YELLOW}[8/9] Restarting services...${NC}"
sudo systemctl start chch-server
sudo systemctl start chch-worker

# Step 9: Health check
echo -e "\n${YELLOW}[9/9] Running health checks...${NC}"
sleep 3

if systemctl is-active --quiet chch-server; then
    echo -e "${GREEN}✓ API Server is running${NC}"
else
    echo -e "${RED}✗ API Server failed to start${NC}"
    echo "Recent logs:"
    journalctl -u chch-server -n 20 --no-pager
    exit 1
fi

if systemctl is-active --quiet chch-worker; then
    echo -e "${GREEN}✓ Replication Worker is running${NC}"
else
    echo -e "${RED}✗ Replication Worker failed to start${NC}"
    echo "Recent logs:"
    journalctl -u chch-worker -n 20 --no-pager
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
echo "Python package: $(pip show ch-ch-changes 2>/dev/null | grep Version || echo 'unknown')"
echo "Status:"
echo "  API Server: $(systemctl is-active chch-server)"
echo "  Worker:     $(systemctl is-active chch-worker)"
echo ""
echo "Useful commands:"
echo "  View logs:        journalctl -u chch-server -f"
echo "                    journalctl -u chch-worker -f"
echo "  Check status:     systemctl status chch-server chch-worker"
echo "  Python shell:     $VENV_DIR/bin/python"
echo "  Converter (Rust): $DEPLOY_DIR/bin/chch-convert"
echo "  List releases:    ls -lh $DEPLOY_DIR/releases/"
