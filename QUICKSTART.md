# 🚀 Quick Start - Deploy to Fly.io in 15 Minutes

## What We've Set Up

✅ **Dockerfile** - Multi-stage build for optimal image size
✅ **fly.toml** - Fly.io configuration (2 vCPU, 2GB RAM, 100GB volume)
✅ **.dockerignore** - Exclude unnecessary files from Docker build
✅ **DEPLOY_FLYIO.md** - Detailed deployment guide

## Your 3-Step Deployment

### 1. Install Fly CLI (1 minute)

```bash
# macOS
brew install flyctl

# Authenticate
flyctl auth login
```

### 2. Create & Deploy App (5 minutes)

```bash
cd /Users/mvexel/dev/changesets-to-parquet

# Launch (creates app but doesn't deploy yet)
flyctl launch --no-deploy

# Create storage volume for parquet data
flyctl volumes create changeset_data --size 100 --region iad

# Deploy!
flyctl deploy
```

### 3. Upload Your Data (varies)

**Option A: Direct sync from local machine (Recommended)**

```bash
# Configure rclone with Fly.io SFTP backend
rclone config
# Choose: sftp
# Host: [your-app-name].fly.dev
# User: root
# Port: 22
# Auth: Use flyctl ssh (leave key_file empty)

# Sync directly from R2 to Fly.io (runs on your machine)
rclone sync r2:your-bucket/changesets/ flyio:/data/changesets/ --progress
```

**Option B: SSH into VM and sync from there**

```bash
# SSH into the VM
flyctl ssh console

# Inside the VM, install rclone
apt update && apt install rclone -y

# Configure rclone for your R2 bucket
rclone config

# Sync your parquet data
rclone sync r2:your-bucket/changesets/ /data/changesets/ --progress

# Exit
exit
```

## Test Your API

```bash
# Get your URL
flyctl info

# Test it
APP_URL="https://your-app-name.fly.dev"

curl $APP_URL/health
curl $APP_URL/meta/tip
curl "$APP_URL/changeset/1.json"
```

## What You Get

- **Live API** at `https://your-app-name.fly.dev`
- **Automatic SSL** certificate
- **Health monitoring** (Fly.io checks /health every 30s)
- **Persistent storage** (100GB volume for parquet data)

## Cost

~$27-30/month for:
- 2 vCPU, 2GB RAM VM (always-on)
- 100GB persistent storage
- Unlimited bandwidth

## Important Files

- **fly.toml** - Fly.io config (edit app name, region, resources)
- **Dockerfile** - Docker build instructions
- **DEPLOY_FLYIO.md** - Complete deployment guide with troubleshooting

## Next Steps After Deploy

1. Test all endpoints
2. Monitor logs: `flyctl logs --follow`
3. Check metrics: `flyctl dashboard`
4. Add custom domain (optional): `flyctl certs add yourdomain.com`

## Problems?

```bash
# Check status
flyctl status

# View logs
flyctl logs

# SSH into VM
flyctl ssh console

# Restart app
flyctl apps restart
```

## Need to Update?

```bash
# After making code changes:
flyctl deploy

# Force rebuild:
flyctl deploy --no-cache
```

---

**Read DEPLOY_FLYIO.md for the complete guide!**
