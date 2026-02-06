# Deployment Scripts

Scripts for native systemd deployment of the changeset server.

## Scripts

### `setup.sh`
**Initial server setup** - Run once on a new VPS to configure everything.

```bash
sudo ./setup.sh
```

This script will:
- Create the `changeset` system user
- Set up directory structure in `/opt/chch`
- Create data directory in `/data/parquet`
- Install the systemd service
- Build and deploy the initial version
- Enable the service to start on boot

### `deploy.sh`
**Deploy a new version** - Use this to deploy code updates.

```bash
./deploy.sh [version]
```

If no version is specified, uses a timestamp (e.g., `20250114_143022`).

This script will:
1. Build a new release binary with `cargo build --release`
2. Create a versioned release directory
3. Copy the binary to the release directory
4. Update symlinks to point to the new version
5. Restart the service
6. Run health checks

Each deployment is kept in `/opt/chch/releases/` for easy rollback.

### `rollback.sh`
**Rollback to a previous version** - Use when a deployment goes wrong.

```bash
./rollback.sh <version>

# Example
./rollback.sh 20250114_143022
```

This script will:
1. Verify the target version exists
2. Ask for confirmation
3. Update symlinks to the target version
4. Restart the service
5. Run health checks

### `healthcheck.sh`
**Monitor service health** - Check if the service is running and responding.

```bash
./healthcheck.sh
```

This script:
- Checks if the systemd service is active
- Tests the `/health` endpoint
- Logs results to syslog
- Optionally sends email alerts (set `ALERT_EMAIL` environment variable)

Use with cron for automated monitoring:
```bash
# Add to crontab
*/5 * * * * /path/to/scripts/healthcheck.sh
```

For email alerts:
```bash
# In crontab
ALERT_EMAIL=admin@example.com
*/5 * * * * /path/to/scripts/healthcheck.sh
```

### `update-data.sh`
**Update parquet data** - Download and process the latest OSM changeset dump with zero-downtime hot swap.

```bash
sudo ./update-data.sh
```

This script:
1. Downloads the latest changeset dump from planet.osm.org
2. Processes it to parquet format (partitioned with indexes)
3. Atomically swaps old data with new data (zero downtime)
4. Restarts the service to pick up new data
5. Runs health checks
6. Cleans up temporary files

**Features:**
- Atomic swap - old data backed up before replacement
- Automatic rollback on failure
- Resume capability for interrupted downloads
- Disk space validation
- Full logging with timestamps

**Resource requirements:**
- ~50GB free disk space minimum
- Processing takes 30-90 minutes depending on hardware
- High memory usage during processing (~6-8GB)

**Schedule with cron:**
```bash
# Weekly update on Sundays at 3 AM
0 3 * * 0 /path/to/chch-convert/scripts/update-data.sh >> /var/log/changeset-update.log 2>&1
```

**Environment variables:**
- `CHANGESET_URL` - Override the download URL (default: planet.osm.org)
- `REPO_DIR` - Override repository location (auto-detected by default)

## Directory Structure

After setup, the deployment directory structure is:

```
/opt/chch/
├── bin/
│   └── chch-server -> ../releases/current/chch-server
├── releases/
│   ├── 20250114_143022/
│   │   └── chch-server
│   ├── 20250114_150301/
│   │   └── chch-server
│   └── current -> 20250114_150301
└── shared/
```

This structure allows for:
- Easy rollbacks (just change the symlink)
- Multiple versions available
- Zero-downtime deployments (build new, switch symlink, restart)

## Common Workflows

### First time setup
```bash
# On the VPS
git clone https://codeberg.org/mvexel/chch-convert.git
cd chch-convert
sudo ./scripts/setup.sh

# Copy parquet data
scp changesets_*.parquet vps:/data/parquet/

# Restart to pick up data
sudo systemctl restart chch-server
```

### Deploying an update
```bash
# On the VPS
cd chch-convert
git pull
./scripts/deploy.sh
```

### Rolling back a bad deployment
```bash
# List available versions
ls /opt/chch/releases/

# Rollback
./scripts/rollback.sh 20250114_143022
```

### Checking health
```bash
# Manual check
./scripts/healthcheck.sh

# Check service status
systemctl status chch-server

# View logs
journalctl -u chch-server -f
```

### Updating parquet data
```bash
# Manual update (downloads and processes latest dump)
sudo ./scripts/update-data.sh

# Or schedule weekly updates with cron
sudo crontab -e
# Add: 0 3 * * 0 /root/chch-convert/scripts/update-data.sh >> /var/log/changeset-update.log 2>&1
```

## Environment Variables

Scripts respect these environment variables:

- `ALERT_EMAIL` - Email address for health check alerts (healthcheck.sh)
- `DATA_PATH` - Path to parquet data (default: `/opt/chch/data/changesets`) - set in systemd service file

## See Also

- [SERVING.md](../docs/SERVING.md) - Full deployment guide
- [systemd/chch-server.service](../systemd/chch-server.service) - systemd service configuration
