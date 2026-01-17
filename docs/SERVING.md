# Hosting the Changeset Server

A guide to hosting the changeset server on a VPS using native deployment (no Docker). This approach offers better performance, simpler debugging, and easier resource management for this single-binary Rust application.

## Why Native Deployment?

For this application, native deployment has several advantages:
- **Performance**: No Docker overhead for file I/O when serving large parquet files
- **Simplicity**: One binary, one systemd service, straightforward debugging
- **Resource efficiency**: Direct access to system resources without container layers
- **Easy recovery**: systemd handles automatic restarts and lifecycle management
- **Transparent monitoring**: Direct access to processes, logs, and system tools

## Prerequisites

- VPS with Ubuntu 20.04+ or similar (Hetzner CPX31 recommended: 4 vCPU, 8GB RAM, 160GB disk)
- Root/sudo access
- Domain name pointed to your server (optional, but recommended for HTTPS)
- SSH access

## Quick Start

For the impatient, here's the TL;DR:

```bash
# On your VPS
git clone https://codeberg.org/mvexel/changesets-to-parquet.git
cd changesets-to-parquet
sudo ./scripts/setup.sh

# Copy your parquet files to /data/parquet
# Then restart: sudo systemctl restart changeset-server
```

## Detailed Setup

### Step 1: Prepare the Server

SSH into your server and update the system:

```bash
# Update system packages
sudo apt update && sudo apt upgrade -y

# Install required dependencies
sudo apt install -y build-essential pkg-config libssl-dev curl git

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
```

### Step 2: Clone the Repository

```bash
cd /tmp
git clone https://codeberg.org/mvexel/changesets-to-parquet.git
cd changesets-to-parquet
```

### Step 3: Run the Setup Script

The setup script will:
- Create the `changeset` system user
- Set up directory structure
- Install the systemd service
- Build and deploy the initial version

```bash
sudo ./scripts/setup.sh
```

### Step 4: Add Your Data

Copy your parquet files to the data directory:

```bash
# Example: Copy from local machine
scp changesets_*.parquet your-server:/data/parquet/

# Or process on the server
# See "Initial Data Load" section below
```

### Step 5: Start the Service

```bash
sudo systemctl restart changeset-server
sudo systemctl status changeset-server
```

### Step 6: Verify It's Working

```bash
# Check service status
systemctl status changeset-server

# Check health endpoint
curl http://localhost:3000/health

# View logs
journalctl -u changeset-server -f
```

## Initial Data Load

You have two options for getting parquet data:

### Option A: Process on the VPS

```bash
# Download the latest changeset dump
cd /tmp
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Build the converter
cargo build --release --bin changesets-to-parquet

# Process (this will take time and memory)
./target/release/changesets-to-parquet \
  --input changesets-latest.osm.bz2 \
  --output /data/parquet/changesets.parquet

# Restart server to pick up new data
sudo systemctl restart changeset-server
```

### Option B: Process on Hetzner Cloud (Recommended)

Use ephemeral Hetzner Cloud servers for heavy processing, then sync to VPS. This is more cost-effective for large datasets.

See [HETZNER.md](HETZNER.md) for full instructions.

## Deployment Operations

### Deploying Updates

```bash
cd /path/to/changesets-to-parquet
git pull
./scripts/deploy.sh
```

The deploy script will:
1. Build a new release binary
2. Create a versioned release directory
3. Update symlinks
4. Restart the service
5. Run health checks

Each deployment is versioned (e.g., `20250114_143022`) and kept in `/opt/changeset-server/releases/` for easy rollback.

### Rolling Back

If a deployment goes wrong, rollback to a previous version:

```bash
# List available versions
ls /opt/changeset-server/releases/

# Rollback to a specific version
./scripts/rollback.sh 20250114_143022
```

### Viewing Logs

```bash
# Follow logs in real-time
journalctl -u changeset-server -f

# Last 100 lines
journalctl -u changeset-server -n 100

# Logs from the last hour
journalctl -u changeset-server --since "1 hour ago"

# Logs for a specific time range
journalctl -u changeset-server --since "2025-01-14 14:00" --until "2025-01-14 15:00"
```

### Checking Service Status

```bash
# Quick status check
systemctl status changeset-server

# Check if running
systemctl is-active changeset-server

# Check resource usage
systemctl status changeset-server | grep -i memory
```

## Setting Up HTTPS with Caddy

For production deployment, you'll want HTTPS. Caddy makes this trivial:

### Install Caddy

```bash
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update
sudo apt install caddy
```

### Configure Caddy

Create `/etc/caddy/Caddyfile`:

```
changesets.yourdomain.com {
    reverse_proxy localhost:3000

    # Enable compression
    encode gzip

    # Serve static files (if you have them)
    handle_path /static/* {
        root * /opt/changeset-server/static
        file_server
    }

    # API endpoints
    handle /* {
        reverse_proxy localhost:3000
    }
}
```

Replace `changesets.yourdomain.com` with your actual domain.

### Start Caddy

```bash
sudo systemctl enable caddy
sudo systemctl start caddy
```

Caddy will automatically obtain and renew Let's Encrypt certificates.

## Monitoring and Alerting

### Health Checks

Set up automated health checks with cron:

```bash
crontab -e
```

Add this line to check every 5 minutes:

```
*/5 * * * * /path/to/changesets-to-parquet/scripts/healthcheck.sh
```

For email alerts, set the `ALERT_EMAIL` environment variable:

```bash
# In crontab
ALERT_EMAIL=admin@example.com
*/5 * * * * /path/to/changesets-to-parquet/scripts/healthcheck.sh
```

### Resource Monitoring

Monitor memory and CPU usage:

```bash
# Real-time resource usage
top -p $(pgrep changeset-server)

# Memory usage
systemctl status changeset-server | grep Memory

# Check systemd resource limits
systemctl show changeset-server | grep -i limit
```

### Log Rotation

systemd/journald handles log rotation automatically. Configure in `/etc/systemd/journald.conf`:

```ini
[Journal]
SystemMaxUse=500M
SystemKeepFree=1G
MaxRetentionSec=7day
```

Then restart journald:

```bash
sudo systemctl restart systemd-journald
```

## Scheduled Updates

To keep your data fresh, schedule regular updates using the `update-data.sh` script. This script:
- Downloads the latest OSM changeset dump
- Processes it to parquet format
- Performs a zero-downtime atomic swap of the data
- Restarts the service
- Validates everything works

### Option 1: Using Cron (Simple)

```bash
# Edit root's crontab
sudo crontab -e

# Add weekly update on Sundays at 3 AM
0 3 * * 0 /root/changesets-to-parquet/scripts/update-data.sh >> /var/log/changeset-update.log 2>&1
```

### Option 2: Using systemd Timers (More robust)

```bash
# Create service file
sudo tee /etc/systemd/system/changeset-update.service > /dev/null << 'EOF'
[Unit]
Description=OSM Changeset Data Update
After=network.target

[Service]
Type=oneshot
User=root
WorkingDirectory=/root/changesets-to-parquet
ExecStart=/root/changesets-to-parquet/scripts/update-data.sh
StandardOutput=append:/var/log/changeset-update.log
StandardError=append:/var/log/changeset-update.log
TimeoutStartSec=6h
EOF

# Create timer
sudo tee /etc/systemd/system/changeset-update.timer > /dev/null << 'EOF'
[Unit]
Description=Weekly OSM Changeset Data Update

[Timer]
OnCalendar=Sun *-*-* 03:00:00
Persistent=true

[Install]
WantedBy=timers.target
EOF

# Enable timer
sudo systemctl daemon-reload
sudo systemctl enable changeset-update.timer
sudo systemctl start changeset-update.timer

# Verify
systemctl list-timers | grep changeset

# View logs
journalctl -u changeset-update.service
```

### Option 3: Hetzner Cloud Processing (Alternative)

Use cron to trigger Hetzner Cloud processing:

```bash
crontab -e
```

Add:

```
0 3 * * 0 /path/to/changesets-to-parquet/scripts/hetzner-process-changesets.sh >> /var/log/changeset-update.log 2>&1
```

## Troubleshooting

### Service Won't Start

```bash
# Check service status
systemctl status changeset-server

# Check recent logs
journalctl -u changeset-server -n 50

# Check if binary is executable
ls -la /opt/changeset-server/bin/changeset-server

# Check data directory permissions
ls -la /data/parquet/
```

### Health Check Fails

```bash
# Check if port 3000 is listening
sudo netstat -tlnp | grep 3000

# Test directly
curl -v http://localhost:3000/health

# Check for errors in logs
journalctl -u changeset-server | grep -i error
```

### High Memory Usage

```bash
# Check current memory usage
systemctl status changeset-server | grep Memory

# Adjust memory limit in service file
sudo systemctl edit changeset-server

# Add:
[Service]
MemoryMax=6G

# Reload and restart
sudo systemctl daemon-reload
sudo systemctl restart changeset-server
```

### Binary Crashes

systemd will automatically restart the service. To investigate:

```bash
# Check crash logs
journalctl -u changeset-server | grep -i "signal\|segfault\|panic"

# Check coredumps (if enabled)
coredumpctl list
coredumpctl info
```

## Directory Structure

After setup, your directory structure will look like:

```
/opt/changeset-server/
├── bin/
│   └── changeset-server -> ../releases/current/changeset-server
├── releases/
│   ├── 20250114_143022/
│   │   └── changeset-server
│   ├── 20250114_150301/
│   │   └── changeset-server
│   └── current -> 20250114_150301
└── shared/

/data/parquet/
├── changesets_2024_01.parquet
├── changesets_2024_02.parquet
└── ...
```

## Security Considerations

1. **Firewall**: Only expose necessary ports
   ```bash
   sudo ufw allow 22/tcp   # SSH
   sudo ufw allow 80/tcp   # HTTP
   sudo ufw allow 443/tcp  # HTTPS
   sudo ufw enable
   ```

2. **Dedicated user**: The service runs as the `changeset` user with limited permissions

3. **systemd security**: The service file includes security hardening:
   - `NoNewPrivileges=true`
   - `ProtectSystem=strict`
   - `ProtectHome=true`
   - `PrivateTmp=true`

4. **Keep system updated**:
   ```bash
   sudo apt update && sudo apt upgrade -y
   ```

5. **Monitor logs**: Regularly check for suspicious activity

## Performance Tuning

### For Large Datasets

Adjust systemd resource limits in `/etc/systemd/system/changeset-server.service`:

```ini
[Service]
MemoryMax=8G
CPUQuota=400%
LimitNOFILE=131072
```

### For High Traffic

Consider adding more workers (if supported by the server code) or placing multiple instances behind a load balancer.

## Backup Strategy

Since the server is read-only (serving parquet files):

1. **Backup parquet data**: Regularly backup `/data/parquet`
   ```bash
   # Example: backup to remote storage
   rclone sync /data/parquet remote:backups/parquet
   ```

2. **Backup configuration**: Keep deployment scripts and service files in git

3. **Document procedures**: Keep this guide updated with any custom changes

## Migration from Docker

If you're migrating from a Docker-based deployment:

1. Stop the Docker containers
2. Copy parquet data to `/data/parquet`
3. Run the native setup script
4. Update your reverse proxy to point to the new setup
5. Remove old Docker volumes/images when confident

## Useful Commands Reference

```bash
# Service management
sudo systemctl start changeset-server
sudo systemctl stop changeset-server
sudo systemctl restart changeset-server
sudo systemctl status changeset-server
sudo systemctl enable changeset-server
sudo systemctl disable changeset-server

# Logs
journalctl -u changeset-server -f                    # Follow logs
journalctl -u changeset-server -n 100                # Last 100 lines
journalctl -u changeset-server --since "1 hour ago"  # Recent logs
journalctl -u changeset-server --since today         # Today's logs

# Deployment
./scripts/deploy.sh                                  # Deploy new version
./scripts/rollback.sh 20250114_143022               # Rollback
./scripts/healthcheck.sh                            # Manual health check

# Monitoring
curl http://localhost:3000/health                    # Health check
curl http://localhost:3000/meta/replication         # Replication state
systemctl show changeset-server | grep Memory       # Memory usage
```

## Getting Help

- Report issues: https://codeberg.org/mvexel/changesets-to-parquet/issues
- Check logs first: `journalctl -u changeset-server -n 100`
- Include version info when reporting issues
