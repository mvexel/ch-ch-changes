# Deployment Scripts

Scripts for native deployment of the changeset server (no Docker).

## Scripts

### `setup.sh`
**Initial server setup** - Run once on a new VPS to configure everything.

```bash
sudo ./setup.sh
```

This script will:
- Create the `changeset` system user
- Set up directory structure in `/opt/changeset-server`
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

Each deployment is kept in `/opt/changeset-server/releases/` for easy rollback.

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

## Directory Structure

After setup, the deployment directory structure is:

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
```

This structure allows for:
- Easy rollbacks (just change the symlink)
- Multiple versions available
- Zero-downtime deployments (build new, switch symlink, restart)

## Common Workflows

### First time setup
```bash
# On the VPS
git clone https://codeberg.org/mvexel/changesets-to-parquet.git
cd changesets-to-parquet
sudo ./scripts/setup.sh

# Copy parquet data
scp changesets_*.parquet vps:/data/parquet/

# Restart to pick up data
sudo systemctl restart changeset-server
```

### Deploying an update
```bash
# On the VPS
cd changesets-to-parquet
git pull
./scripts/deploy.sh
```

### Rolling back a bad deployment
```bash
# List available versions
ls /opt/changeset-server/releases/

# Rollback
./scripts/rollback.sh 20250114_143022
```

### Checking health
```bash
# Manual check
./scripts/healthcheck.sh

# Check service status
systemctl status changeset-server

# View logs
journalctl -u changeset-server -f
```

## Environment Variables

Scripts respect these environment variables:

- `ALERT_EMAIL` - Email address for health check alerts (healthcheck.sh)
- `DATA_PATH` - Path to parquet data (default: `/data/parquet`) - set in systemd service file

## See Also

- [SERVING.md](../docs/SERVING.md) - Full deployment guide
- [systemd/changeset-server.service](../systemd/changeset-server.service) - systemd service configuration
