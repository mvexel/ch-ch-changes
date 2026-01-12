# Quickstart: Deploy on Hetzner

This guide walks you through deploying the OSM Changeset API on a Hetzner VPS using Docker Compose.

## Prerequisites

- Hetzner VPS (CPX31 recommended: 4 vCPU, 8GB RAM, 160GB disk)
- Domain name pointed to your server IP (optional but recommended for HTTPS)
- SSH access to your server

## Step 1: Prepare the Server

SSH into your Hetzner server and run:

```bash
# Update system
apt update && apt upgrade -y

# Install Docker and Docker Compose
curl -fsSL https://get.docker.com | sh
apt install -y docker-compose-plugin

# Create directory structure
mkdir -p /opt/changeset-app
mkdir -p /opt/changeset-data/{changesets,downloads,temp_changesets}
mkdir -p /var/log/changeset

# Configure firewall
ufw allow ssh
ufw allow http
ufw allow https
ufw enable
```

## Step 2: Clone and Configure

```bash
# Clone the repository
cd /opt/changeset-app
git clone https://github.com/yourusername/changesets-to-parquet.git .

# Update Caddyfile with your domain
# Edit the Caddyfile and replace:
# - changesets.yourdomain.com with your actual domain
# - your-email@example.com with your email for Let's Encrypt
nano Caddyfile

# Update docker-compose.yml image registry host
# Example: registry.yourdomain.com -> registry.example.com
nano docker-compose.yml
```

## Step 3: Build and Push Images (on your build machine)

You only need to do this when you release a new version.
Make sure your registry stack is already running and reachable.

```bash
# Build and push images to your registry
docker build -f Dockerfile.server -t registry.yourdomain.com/changeset-api:latest .
docker build -f Dockerfile.builder -t registry.yourdomain.com/changeset-builder:latest .
docker push registry.yourdomain.com/changeset-api:latest
docker push registry.yourdomain.com/changeset-builder:latest
```

## Step 4: Pull and Start Services (on the server)

```bash
# Log in to your registry and pull images
docker login registry.yourdomain.com
docker compose pull
docker compose up -d
```

## Step 5: Initial Data Load

The first time you deploy, you need to download and process the OSM changeset data:

```bash
# This will download ~7.6GB and process to ~15GB Parquet files
# Estimated time: 1-2 hours
docker compose run --rm builder

# Monitor progress
docker compose logs -f changeset-builder
```

## Step 6: Set Up Weekly Automation

Create a systemd service to run the builder weekly:

```bash
# Create service file
cat > /etc/systemd/system/changeset-builder.service << 'EOF'
[Unit]
Description=OSM Changeset Builder
After=docker.service
Requires=docker.service

[Service]
Type=oneshot
User=root
WorkingDirectory=/opt/changeset-app
ExecStart=/usr/bin/docker compose run --rm builder
StandardOutput=append:/var/log/changeset/builder-%Y%m%d.log
StandardError=append:/var/log/changeset/builder-%Y%m%d.log
MemoryMax=7.5G
TimeoutStartSec=6h

[Install]
WantedBy=multi-user.target
EOF

# Create timer file
cat > /etc/systemd/system/changeset-builder.timer << 'EOF'
[Unit]
Description=Weekly OSM Changeset Processing
Requires=changeset-builder.service

[Timer]
OnCalendar=Sun *-*-* 03:00:00
Persistent=true
RandomizedDelaySec=1h

[Install]
WantedBy=timers.target
EOF

# Enable and start timer
systemctl daemon-reload
systemctl enable changeset-builder.timer
systemctl start changeset-builder.timer

# Verify timer is scheduled
systemctl list-timers | grep changeset
```

## Step 7: Verify Deployment

```bash
# Check API health
curl http://localhost:3000/health

# If you configured a domain with HTTPS:
curl https://changesets.yourdomain.com/health

# Check running containers
docker compose ps

# View API logs
docker compose logs api

# Check resource usage
docker stats
```

## Testing Without a Domain

If you don't have a domain set up yet, you can test with the server's IP address:

1. Edit `Caddyfile` and comment out the domain section
2. Uncomment the IP-based configuration:
```
http://YOUR.SERVER.IP.ADDRESS {
    reverse_proxy api:3000
}
```
3. Restart Caddy: `docker compose restart caddy`
4. Access the API at: `http://YOUR.SERVER.IP.ADDRESS/health`

## Monitoring and Maintenance

### Check API Status
```bash
curl https://changesets.yourdomain.com/health
docker compose logs -f api
```

### Check Builder Status
```bash
systemctl status changeset-builder.timer
journalctl -u changeset-builder.service -n 50
```

### Manual Builder Run
```bash
cd /opt/changeset-app
docker compose run --rm builder
```

### View Disk Usage
```bash
df -h /opt/changeset-data
docker system df
```

### View Logs
```bash
# API logs
docker compose logs api

# Caddy logs
docker compose logs caddy

# Builder logs
ls -lh /var/log/changeset/
```

## Troubleshooting

### API won't start
```bash
# Check logs
docker compose logs api

# Verify data directory exists
ls -lh /opt/changeset-data/changesets/

# Rebuild if needed
docker compose build --no-cache api
docker compose up -d api
```

### Caddy SSL issues
```bash
# Check logs
docker compose logs caddy

# Verify DNS is pointing to your server
dig changesets.yourdomain.com

# Ensure ports 80 and 443 are open
ufw status
```

### Builder fails
```bash
# Check disk space
df -h

# Check memory
free -h

# View detailed logs
docker compose run --rm builder
```

## Resource Usage

**Normal operation:**
- API: ~500MB-1GB RAM
- Caddy: ~50-100MB RAM
- Disk: ~15GB for Parquet files

**During weekly builder run:**
- Builder: ~6-7GB RAM
- Temporary disk usage: ~52GB (cleaned up after processing)

## API Endpoints

Once deployed, your API will be available at:

- **Health check:** `GET /health`
- **Latest changeset:** `GET /meta/tip`
- **Coverage info:** `GET /meta/coverage`
- **Single changeset:** `GET /changeset/{id}.json`
- **Daily changesets:** `GET /changesets/day/{YYYY-MM-DD}`
- **Date range:** `GET /changesets/from/{start}/to/{end}`

Example:
```bash
curl https://changesets.yourdomain.com/changeset/1.json
```

## Cost

Running on Hetzner CPX31: ~€13.17/month (~$14.50/month)

## Next Steps

- Set up monitoring (optional)
- Configure backups (optional)
- Set up log rotation
- Consider adding Prometheus/Grafana for metrics

For more detailed information, see the main README.md
