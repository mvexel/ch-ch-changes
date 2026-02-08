#!/usr/bin/env bash
set -euo pipefail

# Deploy script for Python application updates
# Usage: ./deploy.sh [version]

INSTALL_DIR="/opt/chch"
REPO_DIR="$INSTALL_DIR/repo"
VENV_DIR="$INSTALL_DIR/venv"
RELEASES_DIR="$INSTALL_DIR/releases"

VERSION="${1:-$(date +%Y%m%d_%H%M%S)}"
RELEASE_DIR="$RELEASES_DIR/$VERSION"

echo "=== Deploying ch-ch-changes version: $VERSION ==="
echo

# Check if running as root or with correct permissions
if [[ $EUID -ne 0 ]] && [[ ! -w "$INSTALL_DIR" ]]; then
   echo "This script must be run as root or with write access to $INSTALL_DIR"
   exit 1
fi

# Navigate to repo
if [[ ! -d "$REPO_DIR" ]]; then
    echo "Error: Repository not found at $REPO_DIR"
    echo "Run setup.sh first"
    exit 1
fi

cd "$REPO_DIR"

# Pull latest code
echo "Pulling latest code..."
sudo -u chch git fetch --all
sudo -u chch git pull origin python-rewrite || sudo -u chch git pull origin main

# Run tests
echo "Running tests..."
sudo -u chch "$VENV_DIR/bin/python" -m pytest tests/ -q || {
    echo "Tests failed! Aborting deployment."
    exit 1
}

# Create release directory
echo "Creating release $VERSION..."
mkdir -p "$RELEASE_DIR"
sudo -u chch cp -r "$REPO_DIR/src" "$RELEASE_DIR/"
sudo -u chch cp "$REPO_DIR/pyproject.toml" "$RELEASE_DIR/"

# Install/upgrade in venv
echo "Installing dependencies..."
sudo -u chch "$VENV_DIR/bin/pip" install --upgrade pip
sudo -u chch "$VENV_DIR/bin/pip" install -e "$REPO_DIR"

# Update current symlink
echo "Updating 'current' symlink..."
ln -sfn "$VERSION" "$RELEASES_DIR/current"

# Restart services
echo "Restarting services..."
systemctl restart chch-server.service || echo "Warning: Failed to restart chch-server"
systemctl restart chch-worker.service || echo "Warning: Failed to restart chch-worker"

# Wait for services to start
sleep 5

# Health check
echo "Running health checks..."
if curl -f -s http://localhost:3000/health > /dev/null; then
    echo "✓ Server health check passed"

    # Show brief status
    curl -s http://localhost:3000/health | python3 -m json.tool || true
else
    echo "✗ Server health check failed!"
    echo "Check logs: journalctl -u chch-server -n 50"
    exit 1
fi

# Check systemd status
if systemctl is-active --quiet chch-server.service; then
    echo "✓ Server service is running"
else
    echo "✗ Server service is not running"
    systemctl status chch-server.service
    exit 1
fi

if systemctl is-active --quiet chch-worker.service; then
    echo "✓ Worker service is running"
else
    echo "✗ Worker service is not running"
    systemctl status chch-worker.service
    exit 1
fi

echo
echo "=== Deployment successful! ==="
echo "Version: $VERSION"
echo "View logs:"
echo "  Server: journalctl -u chch-server -f"
echo "  Worker: journalctl -u chch-worker -f"
echo

# Cleanup old releases (keep last 5)
echo "Cleaning up old releases..."
cd "$RELEASES_DIR"
ls -t | grep -E '^[0-9]{8}_[0-9]{6}$' | tail -n +6 | xargs -r rm -rf
echo "Done"
