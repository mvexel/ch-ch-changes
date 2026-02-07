# Docker Deployment

**Recommended deployment method** — works on any system with Docker installed.

## Quick Start

### Development (local testing)

```bash
# Build the image
docker build -t chch:latest .

# Run server only
docker run -p 3000:3000 -v ~/data/changesets:/data \
  -e DATA_PATH=/data \
  chch:latest ch_ch_changes.server.app

# Or use docker-compose for both server + worker
docker-compose up -d
```

Access the API at http://localhost:3000

### Production Deployment

```bash
# 1. Clone the repository
git clone https://codeberg.org/mvexel/ch-ch-changes.git
cd ch-ch-changes

# 2. Create .env file
cat > .env <<EOF
DATA_PATH=/opt/chch/data/changesets
PORT=3000
LOG_LEVEL=INFO
REQUIRE_AUTH=false
VERSION=$(date +%Y%m%d_%H%M%S)
EOF

# 3. Build and start services
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d --build

# 4. Check logs
docker-compose logs -f

# 5. Verify health
curl http://localhost:3000/health
```

## Services

The docker-compose setup runs two services:

### `server`
- API server on port 3000
- Reads Parquet files from mounted data volume
- Health check endpoint: `/health`
- Auto-restart on failure

### `worker`
- Replication worker (fetches OSM minutely changesets)
- Writes to delta/ directory
- Runs continuously, polling every 60s by default
- Auto-restart on failure

## Configuration

All configuration via environment variables in `.env` file:

### Server Config
```bash
DATA_PATH=/path/to/data          # Where parquet files live
HOST=0.0.0.0                     # Listen address
PORT=3000                        # HTTP port
LOG_LEVEL=INFO                   # DEBUG, INFO, WARNING, ERROR
REQUIRE_AUTH=false               # Enable API authentication
```

### Worker Config
```bash
DATA_PATH=/path/to/data                    # Where to write parquet files
REPLICATION_START_SEQUENCE=                # Starting sequence (empty = auto-detect)
POLL_INTERVAL_SECONDS=60                   # How often to check for new data
MAX_SEQUENCES_PER_RUN=1000                 # Max sequences per poll
HOURLY_BATCHING=true                       # Batch minutely data into hourly files
ENABLE_COMPACTION=true                     # Merge old hourly files
COMPACTION_AGE_HOURS=24                    # Only compact files older than this
```

## Data Directory Structure

The `DATA_PATH` should contain:

```
/data/
├── base/                          # Base layer (weekly planet dumps)
│   └── year=YYYY/month=MM/*.parquet
├── delta/                         # Delta layer (replication)
│   ├── year=YYYY/month=MM/*.parquet
│   └── state.yaml                 # Replication state
└── metadata.json                  # Dataset statistics (auto-generated)
```

Mount this as a volume: `-v /host/path:/data`

## Common Operations

### View logs
```bash
# All services
docker-compose logs -f

# Server only
docker-compose logs -f server

# Worker only
docker-compose logs -f worker
```

### Restart services
```bash
# All services
docker-compose restart

# Server only
docker-compose restart server

# Worker only
docker-compose restart worker
```

### Stop services
```bash
docker-compose down
```

### Deploy new version
```bash
# Pull latest code
git pull

# Rebuild and restart with zero downtime
docker-compose build
docker-compose up -d
```

### Rollback to previous version
```bash
# Tag images during deployment
docker tag chch:latest chch:20250207_120000

# Rollback
export VERSION=20250207_120000
docker-compose -f docker-compose.yml -f docker-compose.prod.yml up -d
```

## Initial Data Setup

### Option 1: Start with replication from current state
```bash
# Just start the worker - it will bootstrap from OSM's current state
docker-compose up -d worker
```

This will start fetching changesets from OSM's current replication sequence.

### Option 2: Bulk import from planet dump
```bash
# Convert a planet changeset dump to Parquet
docker run -v ~/data/changesets:/data -v ~/downloads:/input \
  chch:latest ch_ch_changes.converter.main \
  --input /input/changesets-latest.osm.bz2 \
  --output-dir /data/base \
  --partition

# Then start services
docker-compose up -d
```

## Monitoring

### Health checks
```bash
# Check if services are running
docker-compose ps

# Check API health endpoint
curl http://localhost:3000/health

# Check replication state
curl http://localhost:3000/meta/state
```

### Resource usage
```bash
# See CPU/memory usage
docker stats

# Set resource limits in docker-compose.prod.yml
```

## Troubleshooting

### Server won't start
```bash
# Check logs
docker-compose logs server

# Common issues:
# - DATA_PATH not mounted or empty
# - Port 3000 already in use
# - Insufficient memory
```

### Worker not fetching data
```bash
# Check logs
docker-compose logs worker

# Common issues:
# - Network connectivity (can't reach planet.openstreetmap.org)
# - Disk space full
# - Permission issues writing to /data
```

### Container keeps restarting
```bash
# View recent logs
docker-compose logs --tail=100 server

# Check exit code
docker-compose ps
```

## Production Best Practices

1. **Use tagged versions** — don't use `latest` in production
2. **Set resource limits** — use `docker-compose.prod.yml` to limit memory/CPU
3. **Configure log rotation** — default is 10MB × 3 files
4. **Monitor disk usage** — Parquet files grow over time
5. **Backup data directory** — `/data` contains all your data
6. **Use reverse proxy** — Put Caddy/Nginx in front for HTTPS
7. **Set up monitoring** — Use `/health` endpoint for uptime checks

## Integration with Caddy

Example Caddyfile for HTTPS:

```
chch.osm.lol {
    reverse_proxy localhost:3000

    encode gzip

    log {
        output file /var/log/caddy/chch.log
    }
}
```

## Upgrading

```bash
# 1. Pull latest code
git pull

# 2. Build new image with version tag
export VERSION=$(date +%Y%m%d_%H%M%S)
docker build -t chch:${VERSION} -t chch:latest .

# 3. Update with zero downtime
docker-compose up -d

# 4. Verify
curl http://localhost:3000/health
curl http://localhost:3000/meta/state
```

## Backup Strategy

```bash
# Backup script
#!/bin/bash
BACKUP_DIR=/backups/$(date +%Y%m%d)
mkdir -p $BACKUP_DIR

# Backup data (incremental rsync)
rsync -av --delete /opt/chch/data/changesets/ $BACKUP_DIR/

# Keep only last 7 days
find /backups -type d -mtime +7 -exec rm -rf {} +
```

## See Also

- [Main README](../../README.md) — Project overview
- [systemd deployment](../systemd/README.md) — Deploy without Docker
- [LOCAL_DEVELOPMENT.md](../../LOCAL_DEVELOPMENT.md) — Development setup
