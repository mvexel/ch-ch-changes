# Deployment Guide

**ch-ch-changes** can be deployed using Docker (recommended) or native systemd installation.

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

# Initial Data Loading

Load complete OSM changeset history (~30-60 minutes):

---

# Serving w Caddy

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