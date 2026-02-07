#!/usr/bin/env bash
set -euo pipefail

# Rollback to a previous release
# Usage: ./rollback.sh <version>

INSTALL_DIR="/opt/chch"
RELEASES_DIR="$INSTALL_DIR/releases"

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <version>"
    echo
    echo "Available versions:"
    ls -1 "$RELEASES_DIR" | grep -E '^[0-9]{8}_[0-9]{6}$' || echo "  (none found)"
    exit 1
fi

VERSION="$1"
RELEASE_DIR="$RELEASES_DIR/$VERSION"

echo "=== Rolling back to version: $VERSION ==="
echo

# Check if version exists
if [[ ! -d "$RELEASE_DIR" ]]; then
    echo "Error: Version $VERSION not found at $RELEASE_DIR"
    echo
    echo "Available versions:"
    ls -1 "$RELEASES_DIR" | grep -E '^[0-9]{8}_[0-9]{6}$'
    exit 1
fi

# Confirm
read -p "Are you sure you want to rollback to $VERSION? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Rollback cancelled"
    exit 0
fi

# Get current version
CURRENT=$(readlink "$RELEASES_DIR/current" 2>/dev/null || echo "unknown")
echo "Current version: $CURRENT"
echo "Target version: $VERSION"

# Update symlink
echo "Updating 'current' symlink..."
ln -sfn "$VERSION" "$RELEASES_DIR/current"

# Restart services
echo "Restarting services..."
systemctl restart chch-server.service
systemctl restart chch-worker.service

# Wait for services to start
sleep 5

# Health check
echo "Running health checks..."
if curl -f -s http://localhost:3000/health > /dev/null; then
    echo "✓ Server health check passed"
    curl -s http://localhost:3000/health | python3 -m json.tool || true
else
    echo "✗ Server health check failed!"
    echo "Check logs: journalctl -u chch-server -n 50"

    # Offer to rollback the rollback
    read -p "Rollback failed. Restore previous version $CURRENT? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        ln -sfn "$CURRENT" "$RELEASES_DIR/current"
        systemctl restart chch-server.service
        systemctl restart chch-worker.service
    fi
    exit 1
fi

# Check systemd status
systemctl is-active --quiet chch-server.service && echo "✓ Server service is running" || echo "✗ Server service failed"
systemctl is-active --quiet chch-worker.service && echo "✓ Worker service is running" || echo "✗ Worker service failed"

echo
echo "=== Rollback successful! ==="
echo "Rolled back from $CURRENT to $VERSION"
echo
