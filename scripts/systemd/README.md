# systemd Deployment (without Docker)

Deploy ch-ch-changes directly on the server using Python virtual environments and systemd services.

**Note:** Docker deployment is recommended for most users (see [../docker/README.md](../docker/README.md)). Use this method if you prefer native installation or can't use Docker.

## Prerequisites

- Ubuntu 20.04+ or Debian 11+ (other Linux distros should work with minor adjustments)
- Python 3.11 or higher
- Root access (sudo)
- At least 4GB RAM (8GB+ recommended for large datasets)

## Quick Start

```bash
# Clone repository
git clone https://codeberg.org/mvexel/ch-ch-changes.git
cd ch-ch-changes

# Run setup (as root)
sudo ./scripts/systemd/setup.sh

# Start services
sudo systemctl start chch-server
sudo systemctl start chch-worker

# Check status
sudo systemctl status chch-server chch-worker

# View logs
journalctl -u chch-server -f
```

## What the Setup Script Does

The `setup.sh` script:

1. ✓ Checks Python version (requires 3.11+)
2. ✓ Installs system dependencies (libxml2, libxslt, build tools)
3. ✓ Creates `chch` system user and group
4. ✓ Creates directory structure in `/opt/chch`:
   ```
   /opt/chch/
   ├── bin/              # Entry point scripts
   ├── repo/             # Git repository
   ├── venv/             # Python virtual environment
   ├── releases/         # Versioned releases
   ├── shared/           # Shared resources
   └── .env              # Configuration
   ```
5. ✓ Creates data directory: `/opt/chch/data/changesets`
6. ✓ Sets up Python virtual environment
7. ✓ Installs ch-ch-changes and dependencies
8. ✓ Installs systemd service files
9. ✓ Enables services for auto-start on boot

## Configuration

Edit `/opt/chch/.env` to configure:

```bash
# Server settings
DATA_PATH=/opt/chch/data/changesets
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
```

After editing, restart services:
```bash
sudo systemctl restart chch-server chch-worker
```

## Service Management

### Start/Stop/Restart

```bash
# Server
sudo systemctl start chch-server
sudo systemctl stop chch-server
sudo systemctl restart chch-server

# Worker
sudo systemctl start chch-worker
sudo systemctl stop chch-worker
sudo systemctl restart chch-worker

# Both
sudo systemctl restart chch-server chch-worker
```

### Enable/Disable Auto-start

```bash
# Enable (start on boot)
sudo systemctl enable chch-server chch-worker

# Disable
sudo systemctl disable chch-server chch-worker
```

### Status and Logs

```bash
# Check status
sudo systemctl status chch-server
sudo systemctl status chch-worker

# View logs (follow mode)
journalctl -u chch-server -f
journalctl -u chch-worker -f

# View recent logs
journalctl -u chch-server -n 100
journalctl -u chch-worker -n 100

# View logs since boot
journalctl -u chch-server -b
```

## Deploying Updates

```bash
# Deploy latest code
cd /opt/chch/repo
git pull
sudo ./scripts/systemd/deploy.sh

# Or specify a version
sudo ./scripts/systemd/deploy.sh 20250207_120000
```

The deploy script:
1. Pulls latest code from git
2. Runs tests (aborts if tests fail)
3. Creates a new versioned release
4. Installs updated dependencies
5. Restarts services
6. Runs health checks
7. Keeps last 5 releases for easy rollback

## Rolling Back

If a deployment causes issues:

```bash
# List available versions
ls /opt/chch/releases/

# Rollback to a specific version
sudo ./scripts/systemd/rollback.sh 20250207_120000
```

The rollback script:
1. Verifies the target version exists
2. Asks for confirmation
3. Updates symlinks
4. Restarts services
5. Runs health checks
6. Offers to revert if health check fails

## Initial Data Setup

### Option 1: Start with replication (recommended)

Just start the worker — it will bootstrap from OSM's current state:

```bash
sudo systemctl start chch-worker
journalctl -u chch-worker -f
```

### Option 2: Bulk import from planet dump

```bash
# Download planet changeset dump
cd /opt/chch/data
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Convert to Parquet
sudo -u chch /opt/chch/venv/bin/python -m ch_ch_changes.converter.main \
  --input changesets-latest.osm.bz2 \
  --output-dir /opt/chch/data/changesets/base \
  --partition

# Start services
sudo systemctl start chch-server chch-worker
```

## Directory Structure

```
/opt/chch/
├── bin/
│   ├── chch-server          # Server entry point
│   └── chch-worker          # Worker entry point
├── repo/                    # Git repository (source code)
├── venv/                    # Python virtual environment
│   └── bin/python           # Python interpreter
├── releases/
│   ├── 20250207_120000/     # Versioned release
│   ├── 20250207_130000/     # Another version
│   └── current -> 20250207_130000  # Symlink to active version
├── shared/                  # Shared resources (if needed)
└── .env                     # Configuration file

/opt/chch/data/changesets/   # Data directory
├── base/                    # Base layer (planet dumps)
│   └── year=YYYY/month=MM/*.parquet
├── delta/                   # Delta layer (replication)
│   ├── year=YYYY/month=MM/*.parquet
│   └── state.yaml           # Replication state
└── metadata.json            # Dataset statistics (auto-generated)
```

## Health Monitoring

### Manual checks

```bash
# API health
curl http://localhost:3000/health

# Replication state
curl http://localhost:3000/meta/state

# Service status
sudo systemctl is-active chch-server chch-worker
```

### Automated monitoring with cron

Create `/opt/chch/bin/healthcheck.sh`:

```bash
#!/bin/bash
if ! curl -f -s http://localhost:3000/health > /dev/null; then
    echo "Health check failed at $(date)"
    sudo systemctl restart chch-server
fi
```

Add to crontab:
```bash
*/5 * * * * /opt/chch/bin/healthcheck.sh
```

## Troubleshooting

### Service won't start

```bash
# Check logs
journalctl -u chch-server -n 50 --no-pager

# Common issues:
# - Permission errors: chown -R chch:chch /opt/chch/data
# - Missing dependencies: cd /opt/chch/repo && sudo -u chch /opt/chch/venv/bin/pip install -e .
# - Port in use: lsof -i :3000
```

### Worker not fetching data

```bash
# Check logs
journalctl -u chch-worker -n 50 --no-pager

# Common issues:
# - Network connectivity: curl -I https://planet.openstreetmap.org
# - Disk space: df -h /opt/chch/data
# - Permission errors: chown -R chch:chch /opt/chch/data
```

### Tests fail during deployment

```bash
# Run tests manually to see details
cd /opt/chch/repo
sudo -u chch /opt/chch/venv/bin/python -m pytest tests/ -v

# Install test dependencies if missing
sudo -u chch /opt/chch/venv/bin/pip install pytest pytest-asyncio
```

### Out of memory

```bash
# Check memory usage
free -h
ps aux | grep python

# Reduce DuckDB memory usage by editing /opt/chch/.env:
echo "DUCKDB_MEMORY_LIMIT=2GB" | sudo tee -a /opt/chch/.env
sudo systemctl restart chch-server
```

## Security Hardening

The systemd service files include these security features:

- `NoNewPrivileges=true` — Prevents privilege escalation
- `PrivateTmp=true` — Isolates /tmp
- `ProtectSystem=strict` — Read-only filesystem except data dirs
- `ProtectHome=true` — No access to user home directories
- `ReadWritePaths=` — Explicitly allows only data directories

To further harden:

```bash
# Run service in a network namespace (if only local access needed)
sudo nano /etc/systemd/system/chch-server.service
# Add: PrivateNetwork=true

# Limit memory usage
sudo systemctl edit chch-server
# Add:
# [Service]
# MemoryMax=4G

sudo systemctl daemon-reload
sudo systemctl restart chch-server
```

## Uninstall

```bash
# Stop and disable services
sudo systemctl stop chch-server chch-worker
sudo systemctl disable chch-server chch-worker

# Remove systemd files
sudo rm /etc/systemd/system/chch-*.service
sudo systemctl daemon-reload

# Remove application (keeps data)
sudo rm -rf /opt/chch

# Remove data (if desired)
sudo rm -rf /opt/chch/data

# Remove user
sudo userdel chch
```

## Performance Tuning

### For large datasets (>100M changesets)

Edit `/opt/chch/.env`:

```bash
# Increase DuckDB memory limit
DUCKDB_MEMORY_LIMIT=8GB

# Increase worker batch size
MAX_SEQUENCES_PER_RUN=5000

# Enable aggressive compaction
ENABLE_COMPACTION=true
COMPACTION_AGE_HOURS=12
```

### For low-memory systems (<4GB RAM)

```bash
# Reduce memory usage
DUCKDB_MEMORY_LIMIT=1GB
MAX_SEQUENCES_PER_RUN=500
```

## Backup and Restore

### Backup

```bash
#!/bin/bash
# backup.sh
BACKUP_DIR=/backups/chch/$(date +%Y%m%d)
mkdir -p $BACKUP_DIR

# Backup data (rsync with hardlinks for space efficiency)
rsync -a --link-dest=/backups/chch/latest \
  /opt/chch/data/ $BACKUP_DIR/

# Update 'latest' symlink
ln -sfn $BACKUP_DIR /backups/chch/latest

# Keep only last 7 days
find /backups/chch -maxdepth 1 -type d -mtime +7 -exec rm -rf {} +
```

### Restore

```bash
# Stop services
sudo systemctl stop chch-server chch-worker

# Restore data
sudo rsync -a /backups/chch/20250207/ /opt/chch/data/

# Fix permissions
sudo chown -R chch:chch /opt/chch/data

# Start services
sudo systemctl start chch-server chch-worker
```

## Comparison: systemd vs Docker

| Feature | systemd | Docker |
|---------|---------|--------|
| **Setup complexity** | Moderate | Simple |
| **Dependency management** | Manual (apt + pip) | Automated (Dockerfile) |
| **Isolation** | Process-level | Container-level |
| **Memory overhead** | Lower (~50MB) | Slightly higher (~100MB) |
| **Portability** | Linux-specific | Any Docker host |
| **Rollback** | Manual script | Image tags |
| **Best for** | Single server, low overhead | Multi-server, reproducible deploys |

## See Also

- [Main README](../../README.md) — Project overview
- [Docker deployment](../docker/README.md) — Recommended deployment method
- [LOCAL_DEVELOPMENT.md](../../LOCAL_DEVELOPMENT.md) — Development setup
