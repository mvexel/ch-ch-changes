# Deployment Guide

**ch-ch-changes** can be deployed using Docker (recommended) or native systemd installation.

## Quick Decision Guide

- **Use Docker if:** You want the easiest setup, portability, and isolation
- **Use systemd if:** You need maximum performance or can't use Docker

---

## Table of Contents

1. [Docker Deployment (Recommended)](#docker-deployment-recommended)
2. [Systemd Deployment (Advanced)](#systemd-deployment-advanced)
3. [Initial Data Loading](#initial-data-loading)
4. [Operations](#operations)
5. [Monitoring](#monitoring)
6. [Troubleshooting](#troubleshooting)

---

# Docker Deployment (Recommended)

Works on any system with Docker installed. Provides isolation, easy updates, and portable deployment.

## Prerequisites

- Docker 20.10+
- Docker Compose 2.0+
- 20GB+ free disk space (more for historical data)
- 4GB+ RAM

## Quick Start

```bash
# 1. Clone repository
git clone https://codeberg.org/mvexel/ch-ch-changes.git
cd ch-ch-changes

# 2. Create data directory
mkdir -p ~/data/chch

# 3. Load initial data (REQUIRED - see "Initial Data Loading" section)
./scripts/docker/init-data.sh

# 4. Start services
docker-compose up -d

# 5. Verify
docker-compose logs -f
curl http://localhost:3000/health
```

## Configuration

Create a `.env` file in the project root:

```bash
# Required
DATA_PATH=~/data/chch

# Optional
PORT=3000
LOG_LEVEL=INFO
REQUIRE_AUTH=false
POLL_INTERVAL_SECONDS=60
HOURLY_BATCHING=true
ENABLE_COMPACTION=true
COMPACTION_AGE_HOURS=24
```

## Services

The Docker setup runs two services:

### Server (`chch-server`)
- FastAPI application serving HTTP API
- Port: 3000 (configurable)
- Queries DuckDB over Parquet files
- Read-only access to data volume

### Worker (`chch-worker`)
- Replication worker fetching new changesets
- Polls OSM every 60s (configurable)
- Writes to delta layer
- Updates metadata automatically

## Production Deployment

For production, use the production overrides:

```bash
# Set version tag
export VERSION=$(date +%Y%m%d_%H%M%S)

# Build with version tag
docker build -t chch:${VERSION} -t chch:latest .

# Start with production settings
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d

# Monitor
docker-compose logs -f
```

Production settings include:
- Memory limits (4GB server, 2GB worker)
- CPU quotas (2 cores server, 1 core worker)
- Log rotation (10MB × 3 files)

## Updates

```bash
# Pull latest code
git pull

# Rebuild and restart (zero downtime)
docker-compose build
docker-compose up -d

# Verify
curl http://localhost:3000/health
```

## Rollback

```bash
# Tag images during deployment
docker tag chch:latest chch:backup-$(date +%Y%m%d)

# Rollback to specific version
export VERSION=20260207_120000
docker-compose up -d
```

---

# Systemd Deployment (Advanced)

Native deployment for maximum performance. Requires manual setup but provides tighter integration with the OS.

## Prerequisites

- Ubuntu 20.04+ or Debian 11+ (or compatible)
- Python 3.11+
- Rust 1.75+ (for initial data converter)
- Root/sudo access
- 20GB+ free disk space
- 4GB+ RAM

## Quick Start

```bash
# 1. Clone repository
git clone https://codeberg.org/mvexel/ch-ch-changes.git
cd ch-ch-changes

# 2. Run setup (creates user, directories, services)
sudo ./scripts/setup.sh

# 3. Load initial data (REQUIRED - see "Initial Data Loading" section)
./scripts/init-data.sh

# 4. Start services
sudo systemctl start chch-server chch-worker

# 5. Verify
systemctl status chch-server chch-worker
curl http://localhost:3000/health
```

## What Setup Does

The `setup.sh` script:
1. Creates `chch` system user
2. Sets up directories:
   - `/opt/chch/` - Application and virtual environment
   - `/opt/chch/data/` - Parquet data files
   - `/var/lib/chch/` - Runtime state (auth DB, etc.)
3. Installs systemd services
4. Checks Python 3.11+ and optionally installs Rust
5. Builds and installs Python package

## Directory Structure

```
/opt/chch/
├── bin/
│   └── chch-convert           # Rust converter for planet dumps
├── venv/                       # Python virtual environment
│   ├── bin/
│   │   ├── chch-server        # FastAPI server
│   │   └── chch-worker        # Replication worker
│   └── lib/...
├── data/                       # Parquet data files
│   ├── base/                   # Base layer (planet dump)
│   ├── delta/                  # Delta layer (replication)
│   ├── metadata.json           # Dataset statistics
│   └── state.yaml              # Replication state
├── releases/
│   └── current_version         # Version tracking
└── .env                        # Configuration

/var/lib/chch/                  # Runtime data
└── auth.db                     # Auth database (if enabled)
```

## Configuration

Edit `/opt/chch/.env`:

```bash
# Data
DATA_PATH=/opt/chch/data

# Server (optional)
OSM_OAUTH_CLIENT_ID=
OSM_OAUTH_CLIENT_SECRET=
OSM_OAUTH_REDIRECT_URL=
AUTH_KEY_SECRET=
AUTH_DB_PATH=/var/lib/chch/auth.db
AUTH_DEFAULT_DAILY_LIMIT=0
AUTH_DEFAULT_MONTHLY_LIMIT=0
REQUIRE_AUTH=true

# Worker (optional)
POLL_INTERVAL_SECONDS=60
CHANGESET_REPLICATION_URL=https://planet.openstreetmap.org/replication/changesets/
HOME=/var/lib/chch
ENABLE_COMPACTION=true
COMPACTION_AGE_HOURS=24
```

## Updates

```bash
# Pull latest code
cd /path/to/ch-ch-changes
git pull

# Deploy update
./scripts/deploy.sh

# Services restart automatically
```

The deploy script:
1. Builds Python package
2. Updates virtual environment
3. Optionally rebuilds Rust converter
4. Restarts services
5. Runs health checks

---

# Initial Data Loading

**CRITICAL:** Services require data before they can start. Choose one option:

## Option 1: Planet Dump Import (Recommended)

Load complete OSM changeset history (~30-60 minutes):

### Docker

```bash
./scripts/docker/init-data.sh
```

Options:
- `--input FILE` - Use existing changeset file
- `--skip-download` - Skip download if file exists
- `--keep` - Keep downloaded file after conversion
- `--data-dir DIR` - Custom output directory

### Systemd

```bash
./scripts/init-data.sh
```

Same options as Docker version.

## Option 2: Start from Current (Minimal)

Start fetching changesets from now (no historical data):

### Docker

```bash
# Just start the worker - it will bootstrap
docker-compose up -d worker

# Wait a few minutes for data to accumulate
sleep 180

# Start the server
docker-compose up -d server
```

### Systemd

```bash
# Start worker only
sudo systemctl start chch-worker

# Wait for data
sleep 180

# Start server
sudo systemctl start chch-server
```

## Option 3: Copy Existing Data

If you already have Parquet files:

### Docker

```bash
cp -r /path/to/existing/data/* ~/data/chch/
docker-compose up -d
```

### Systemd

```bash
sudo cp -r /path/to/existing/data/* /opt/chch/data/
sudo chown -R chch:chch /opt/chch/data
sudo systemctl start chch-server chch-worker
```

---

# Operations

## Common Commands

### Docker

```bash
# View logs
docker-compose logs -f
docker-compose logs -f server
docker-compose logs -f worker

# Restart services
docker-compose restart
docker-compose restart server
docker-compose restart worker

# Stop services
docker-compose down

# Check status
docker-compose ps
docker stats

# Shell access
docker-compose exec server bash
```

### Systemd

```bash
# View logs
journalctl -u chch-server -f
journalctl -u chch-worker -f

# Restart services
sudo systemctl restart chch-server
sudo systemctl restart chch-worker

# Stop services
sudo systemctl stop chch-server chch-worker

# Check status
systemctl status chch-server chch-worker

# Python shell
source /opt/chch/venv/bin/activate
python
```

## Data Integrity Check

Verify parquet files, check for gaps, and view replication status:

```bash
# Docker
docker-compose exec server bash -c "apt-get update && apt-get install -y duckdb"
docker cp scripts/check-integrity.sh chch-server:/tmp/
docker-compose exec server bash /tmp/check-integrity.sh /data

# Systemd
./scripts/check-integrity.sh /opt/chch/data
```

---

# Monitoring

## Health Checks

### API Health Endpoint

```bash
curl http://localhost:3000/health

# Expected response:
# {"status":"ok"}
```

### Replication Status

```bash
curl http://localhost:3000/meta/state

# Shows:
# - Latest changeset ID
# - Replication sequence
# - Data coverage
```

## Automated Monitoring

### Docker

Use Docker's built-in healthcheck (already configured in docker-compose.yml):

```bash
# Check health status
docker-compose ps

# View health check logs
docker inspect chch-server | jq '.[0].State.Health'
```

### Systemd

Set up cron-based health checks:

```bash
# Add to crontab
*/5 * * * * /path/to/ch-ch-changes/scripts/healthcheck.sh

# For email alerts
ALERT_EMAIL=admin@example.com
*/5 * * * * /path/to/ch-ch-changes/scripts/healthcheck.sh
```

## Resource Monitoring

### Docker

```bash
# Real-time stats
docker stats

# Logs with timestamps
docker-compose logs -f --timestamps
```

### Systemd

```bash
# Service resource usage
systemctl status chch-server | grep Memory
systemctl status chch-worker | grep Memory

# Detailed resource info
systemctl show chch-server | grep -E 'Memory|CPU'
```

---

# Troubleshooting

## Services Won't Start

### Docker

```bash
# Check logs
docker-compose logs server
docker-compose logs worker

# Common issues:
# - DATA_PATH not mounted: Check docker-compose.yml volumes
# - Port conflict: Change PORT in .env
# - No data: Run init-data.sh first
```

### Systemd

```bash
# Check logs
journalctl -u chch-server -n 50
journalctl -u chch-worker -n 50

# Common issues:
# - No data: Run init-data.sh first
# - Permission issues: Check ownership of /opt/chch/data
# - Python not found: Check /opt/chch/venv/bin/python exists
```

## No Data / Empty Responses

**Cause:** Services started before data was loaded

**Solution:**

```bash
# Docker
docker-compose down
./scripts/docker/init-data.sh
docker-compose up -d

# Systemd
sudo systemctl stop chch-server chch-worker
./scripts/init-data.sh
sudo systemctl start chch-server chch-worker
```

## Worker Not Fetching

```bash
# Check worker logs
docker-compose logs worker  # Docker
journalctl -u chch-worker -f  # Systemd

# Common issues:
# - Network: Can't reach planet.openstreetmap.org
# - Disk full: Check available space
# - Permission: Check write access to data directory
```

## High Memory Usage

### Docker

Edit `docker-compose.prod.yml`:

```yaml
services:
  server:
    deploy:
      resources:
        limits:
          memory: 8G  # Increase from 4G
```

### Systemd

Edit service files:

```bash
sudo systemctl edit chch-server

# Add:
[Service]
MemoryMax=8G
```

Restart:

```bash
sudo systemctl daemon-reload
sudo systemctl restart chch-server
```

## Data Gaps / Missing Changesets

```bash
# Run integrity check
./scripts/check-integrity.sh /opt/chch/data  # Systemd
# Or for Docker: docker-compose exec server ...

# If gaps found:
# 1. Re-run init-data.sh to reload base layer
# 2. Worker will fill delta gaps automatically
```

---

# HTTPS / Reverse Proxy

## With Caddy (Recommended)

### Install Caddy

```bash
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | \
  sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | \
  sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update
sudo apt install caddy
```

### Configure

Edit `/etc/caddy/Caddyfile`:

```
chch.yourdomain.com {
    reverse_proxy localhost:3000
    encode gzip

    log {
        output file /var/log/caddy/chch.log
    }
}
```

### Start

```bash
sudo systemctl enable caddy
sudo systemctl start caddy
```

Caddy automatically obtains and renews Let's Encrypt certificates.

## With Nginx

```nginx
server {
    listen 80;
    server_name chch.yourdomain.com;

    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

Use certbot for HTTPS:

```bash
sudo apt install certbot python3-certbot-nginx
sudo certbot --nginx -d chch.yourdomain.com
```

---

# Performance Tuning

## For Large Datasets

### Docker

Use `docker-compose.prod.yml` with increased limits:

```yaml
services:
  server:
    deploy:
      resources:
        limits:
          memory: 16G
          cpus: '4.0'
```

### Systemd

Adjust resource limits:

```bash
sudo systemctl edit chch-server

# Add:
[Service]
MemoryMax=16G
CPUQuota=400%
LimitNOFILE=131072
```

## For High Traffic

- Run multiple server instances behind a load balancer
- Use read-only replicas of the data directory
- Enable query result caching (if implemented)

---

# Backup Strategy

## What to Backup

1. **Data directory** - Contains all parquet files
2. **Configuration** - `.env` file and service configurations
3. **Auth database** - `/var/lib/chch/auth.db` (if auth enabled)

## Backup Script Example

```bash
#!/bin/bash
BACKUP_DIR=/backups/$(date +%Y%m%d)
DATA_DIR=/opt/chch/data  # Or ~/data/chch for Docker

mkdir -p $BACKUP_DIR

# Incremental backup with rsync
rsync -av --delete $DATA_DIR/ $BACKUP_DIR/

# Keep last 7 days
find /backups -type d -mtime +7 -exec rm -rf {} +
```

## Restoration

```bash
# Stop services
docker-compose down  # Docker
sudo systemctl stop chch-server chch-worker  # Systemd

# Restore data
rsync -av /backups/20260207/ ~/data/chch/  # Docker
rsync -av /backups/20260207/ /opt/chch/data/  # Systemd

# Fix ownership (systemd only)
sudo chown -R chch:chch /opt/chch/data

# Restart
docker-compose up -d  # Docker
sudo systemctl start chch-server chch-worker  # Systemd
```

---

# Security Best Practices

1. **Firewall** - Only expose necessary ports:
   ```bash
   sudo ufw allow 22/tcp   # SSH
   sudo ufw allow 80/tcp   # HTTP
   sudo ufw allow 443/tcp  # HTTPS
   sudo ufw enable
   ```

2. **Keep Updated**:
   ```bash
   # Docker: rebuild images regularly
   docker-compose build

   # Systemd: update system and application
   sudo apt update && sudo apt upgrade
   ./scripts/deploy.sh
   ```

3. **Enable Authentication** - Set `REQUIRE_AUTH=true` in production

4. **Use HTTPS** - Always use reverse proxy with TLS in production

5. **Limit Resources** - Use memory/CPU limits to prevent DoS

6. **Monitor Logs** - Watch for suspicious activity

---

# Getting Help

- **Documentation**: Check README.md and this guide
- **Issues**: https://codeberg.org/mvexel/ch-ch-changes/issues
- **Logs**: Always check logs first before reporting issues

When reporting issues, include:
- Deployment method (Docker/systemd)
- OS and version
- Error messages from logs
- Steps to reproduce
