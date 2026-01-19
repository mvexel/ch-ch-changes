# Hosting the Changeset Server

## Prerequisites

- VPS with Ubuntu 20.04+ or similar (Hetzner CPX31 recommended: 4 vCPU, 8GB RAM, 160GB disk)
- Root/sudo access
- Domain name pointed to your server (optional, but recommended for HTTPS)
- SSH access

## Quick Start

### Step 1: Prepare the Server

SSH into your server, update the system, and install deps & rust:

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y build-essential pkg-config libssl-dev curl git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
```

### Step 2: Clone the Repository

```bash
cd /tmp
git clone https://codeberg.org/mvexel/changesets-to-parquet.git
cd changesets-to-parquet
```

### Configure

Edit the systemd service definition.

- `OSM_OAUTH_CLIENT_ID` - OSM OAuth2 client ID
- `OSM_OAUTH_CLIENT_SECRET` - OSM OAuth2 client secret
- `OSM_OAUTH_REDIRECT_URL` - e.g. `https://changesets.yourdomain.com/auth/osm/callback`
- `AUTH_KEY_SECRET` - random secret used to hash API keys (create with `openssl rand -hex 32`)
- `AUTH_DB_PATH` - path to the local SQLite auth DB (e.g. `/var/lib/changeset-server/auth.sqlite`)

Optional overrides:

- `DATA_PATH` - changeset parquet data root path.
- `RUST_LOG` - Rust log level
- `HOME` - binary home dir
- `AUTH_DEFAULT_DAILY_LIMIT` - default daily request limit for new users (omit or `0` for unlimited)
- `AUTH_DEFAULT_MONTHLY_LIMIT` - default monthly request limit for new users (omit or `0` for unlimited)
- `OSM_OAUTH_SCOPE` (default: `read_prefs`)
- `OSM_OAUTH_AUTHORIZE_URL`
- `OSM_OAUTH_TOKEN_URL`
- `OSM_OAUTH_USER_DETAILS_URL`

To set per-user limits later, update the local SQLite DB directly

### Step 3: Run the Setup Script

The setup script will:
- Create the `changeset` system user
- Set up directory structure
- Install the systemd service
- Build and deploy the initial version

```bash
sudo ./scripts/setup.sh
```

### Step 4: Initial data seed

See [HETZNER.md](HETZNER.md) for full instructions.

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


### Authentication Configuration



```bash
sqlite3 /var/lib/changeset-server/auth.sqlite \
  "UPDATE users SET daily_limit=5000, monthly_limit=100000 WHERE osm_uid=12345;"
```



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
