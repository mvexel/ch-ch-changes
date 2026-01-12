# Hosting the server

**THIS IS UNFINISHED**

A guide to hosting the changeset server on a Hetzner VPS. (Other Ubuntu LTS based VPS will work equally well of course)

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
```

## Step 2: Clone and Configure

```bash
# Clone the repository
cd /opt/changeset-app
git clone https://codeberg.org/mvexel/changesets-to-parquet.git .

# Create Caddyfile from example
cp Caddyfile.example Caddyfile

# Edit Caddyfile with your domain and email
# Replace:
# - changesets.yourdomain.com with your actual domain
# - your-email@example.com with your email for Let's Encrypt
nano Caddyfile

# Customize homepage with your domain
# Replace DOMAIN placeholder in static/index.html
sed -i 's/DOMAIN/changesets.yourdomain.com/g' static/index.html
```

## Step 3: Build Images (on the server)

Build the Docker images directly on your VPS:

```bash
# Build API server image
docker compose build api

# Build builder image
docker compose build builder
```

## Step 4: Start Services (on the server)

```bash
# Start the services
docker compose up -d
```

## Step 5: Initial Data Load

You have two options for processing the changeset data:

```bash
docker compose run --rm builder
docker compose logs -f changeset-builder
```

This will both download and process the latest changeset dump.

======================================

### Option B: Process on AWS

Use ephemeral AWS EC2 instances for processing heavy workloads, then sync to VPS.

....setup stuff ......

```bash
cd /opt/changeset-app
./scripts/aws-process-changesets.sh
```

....why would you do this....

systemd service to schedule

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

or: 
Use cron to schedule AWS processing (cheaper for large datasets):

```bash
# Edit crontab
crontab -e

# Add weekly processing on Sundays at 3 AM
0 3 * * 0 /opt/changeset-app/scripts/aws-process-changesets.sh >> /var/log/changeset-aws.log 2>&1
```

# Verify

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
