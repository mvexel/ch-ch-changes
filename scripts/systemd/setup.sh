#!/usr/bin/env bash
set -euo pipefail

# Setup script for native Python deployment with systemd
# Run as root: sudo ./setup.sh

# Configuration
APP_USER="chch"
APP_GROUP="chch"
INSTALL_DIR="/opt/chch"
DATA_DIR="/opt/chch/data/changesets"
VENV_DIR="$INSTALL_DIR/venv"
REPO_URL="https://codeberg.org/mvexel/ch-ch-changes.git"

echo "=== ch-ch-changes systemd setup ==="
echo

# Check if running as root
if [[ $EUID -ne 0 ]]; then
   echo "This script must be run as root (use sudo)"
   exit 1
fi

# Check Python version
if ! command -v python3 &> /dev/null; then
    echo "Python 3 is not installed. Installing..."
    apt-get update
    apt-get install -y python3 python3-pip python3-venv
fi

PYTHON_VERSION=$(python3 -c 'import sys; print(".".join(map(str, sys.version_info[:2])))')
echo "Found Python $PYTHON_VERSION"

if [[ $(echo "$PYTHON_VERSION < 3.11" | bc -l) -eq 1 ]]; then
    echo "Error: Python 3.11 or higher is required (found $PYTHON_VERSION)"
    exit 1
fi

# Install system dependencies for lxml and DuckDB
echo "Installing system dependencies..."
apt-get update
apt-get install -y \
    libxml2 \
    libxslt1.1 \
    libxml2-dev \
    libxslt1-dev \
    build-essential \
    curl \
    git

# Create system user
if ! id "$APP_USER" &>/dev/null; then
    echo "Creating system user: $APP_USER"
    useradd --system --user-group --home-dir "$INSTALL_DIR" --shell /bin/bash "$APP_USER"
else
    echo "User $APP_USER already exists"
fi

# Create directories
echo "Creating directory structure..."
mkdir -p "$INSTALL_DIR"/{releases,shared,venv,bin}
mkdir -p "$DATA_DIR"/{base,delta}
chown -R "$APP_USER:$APP_GROUP" "$INSTALL_DIR"
chown -R "$APP_USER:$APP_GROUP" "$DATA_DIR"

# Clone repository if not present
if [[ ! -d "$INSTALL_DIR/repo" ]]; then
    echo "Cloning repository..."
    sudo -u "$APP_USER" git clone "$REPO_URL" "$INSTALL_DIR/repo"
else
    echo "Repository already cloned, pulling latest..."
    cd "$INSTALL_DIR/repo"
    sudo -u "$APP_USER" git pull
fi

# Create Python virtual environment
echo "Creating Python virtual environment..."
sudo -u "$APP_USER" python3 -m venv "$VENV_DIR"

# Install the application
echo "Installing ch-ch-changes..."
cd "$INSTALL_DIR/repo"
sudo -u "$APP_USER" "$VENV_DIR/bin/pip" install --upgrade pip
sudo -u "$APP_USER" "$VENV_DIR/bin/pip" install -e .

# Create entry point scripts
echo "Creating entry point scripts..."
cat > "$INSTALL_DIR/bin/chch-server" <<EOF
#!/bin/bash
source $VENV_DIR/bin/activate
exec python -m ch_ch_changes.server.app
EOF

cat > "$INSTALL_DIR/bin/chch-worker" <<EOF
#!/bin/bash
source $VENV_DIR/bin/activate
exec python -m ch_ch_changes.worker.main
EOF

chmod +x "$INSTALL_DIR/bin/chch-server"
chmod +x "$INSTALL_DIR/bin/chch-worker"

# Create default .env file
if [[ ! -f "$INSTALL_DIR/.env" ]]; then
    echo "Creating default .env file..."
    cat > "$INSTALL_DIR/.env" <<EOF
# ch-ch-changes configuration
DATA_PATH=$DATA_DIR
HOST=0.0.0.0
PORT=3000
LOG_LEVEL=INFO
REQUIRE_AUTH=false

# Worker settings
POLL_INTERVAL_SECONDS=60
MAX_SEQUENCES_PER_RUN=1000
HOURLY_BATCHING=true
ENABLE_COMPACTION=true
COMPACTION_AGE_HOURS=24
EOF
    chown "$APP_USER:$APP_GROUP" "$INSTALL_DIR/.env"
fi

# Install systemd service files
echo "Installing systemd services..."
cp "$INSTALL_DIR/repo/systemd/chch-server.service" /etc/systemd/system/
cp "$INSTALL_DIR/repo/systemd/chch-worker.service" /etc/systemd/system/

# Update service files to use venv
sed -i "s|ExecStart=/opt/chch/bin/chch-server|ExecStart=$INSTALL_DIR/bin/chch-server|" /etc/systemd/system/chch-server.service
sed -i "s|ExecStart=/opt/chch/bin/chch-worker|ExecStart=$INSTALL_DIR/bin/chch-worker|" /etc/systemd/system/chch-worker.service

# Reload systemd
systemctl daemon-reload

# Enable services
echo "Enabling services..."
systemctl enable chch-server.service
systemctl enable chch-worker.service

echo
echo "=== Setup complete! ==="
echo
echo "Next steps:"
echo "1. Edit configuration: sudo nano $INSTALL_DIR/.env"
echo "2. Start server: sudo systemctl start chch-server"
echo "3. Start worker: sudo systemctl start chch-worker"
echo "4. Check status: sudo systemctl status chch-server chch-worker"
echo "5. View logs: journalctl -u chch-server -f"
echo
echo "Health check: curl http://localhost:3000/health"
echo
