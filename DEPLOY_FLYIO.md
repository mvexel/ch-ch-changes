# Deploy Changeset Historic API to Fly.io

This guide will get your API live in ~15 minutes.

## Prerequisites

- Fly.io account (sign up at https://fly.io/app/sign-up)
- Parquet data files ready (or will generate them)

## Step 1: Install Fly CLI

```bash
# macOS
brew install flyctl

# Or via install script
curl -L https://fly.io/install.sh | sh

# Verify installation
flyctl version
```

## Step 2: Authenticate

```bash
flyctl auth login
# Opens browser for authentication
```

## Step 3: Launch the App

```bash
# In your project directory
cd /Users/mvexel/dev/changesets-to-parquet

# Create the app (this will use fly.toml config)
flyctl launch --no-deploy

# Answer the prompts:
# - App name: (press Enter to use 'changeset-api' or choose your own)
# - Region: (choose closest to you, default is 'iad' - US East)
# - PostgreSQL: No
# - Redis: No
```

This creates your app but doesn't deploy yet.

## Step 4: Create Persistent Volume for Data

```bash
# Create a 100GB volume for parquet files
flyctl volumes create changeset_data --size 100 --region iad

# Verify it was created
flyctl volumes list
```

**Note:** Change `iad` to your chosen region if different.

## Step 5: Deploy the API

```bash
# Build and deploy
flyctl deploy

# This will:
# 1. Build the Docker image (takes 5-10 min first time)
# 2. Push to Fly.io registry
# 3. Start the app
# 4. Run health checks
```

## Step 6: Verify Deployment

```bash
# Check if it's running
flyctl status

# View logs
flyctl logs

# Open in browser (will hit /health endpoint initially)
flyctl open
```

Your API should now be live at `https://changeset-api.fly.dev` (or your custom name).

## Step 7: Upload Parquet Data

You need to get your parquet files onto the Fly.io volume.

### Option A: Using flyctl ssh (Small datasets)

```bash
# Connect to the VM
flyctl ssh console

# Inside the VM:
cd /data/changesets

# Use curl/wget to download from R2 or your source
# Example:
apt update && apt install rclone -y
rclone config  # Configure your R2 connection
rclone sync r2:your-bucket/data/ /data/changesets/ --progress

# Exit
exit
```

### Option B: Generate on the VM (If you have the OSM dump)

```bash
# SSH into the VM
flyctl ssh console

# Install build tools if needed
apt update && apt install -y wget bzip2

# Download OSM changeset dump
cd /tmp
wget https://planet.osm.org/planet/changesets-latest.osm.bz2

# Copy the converter binary (it's already in the image if we include it)
# Or build it on the VM
# Then run:
changeset-server --help  # Check if converter is available

# If not, you'll need to add the converter to the Docker image
```

### Option C: Pre-build and upload to R2, then sync (Recommended)

```bash
# On your local machine:
# 1. Generate parquet files locally using your existing tool
./target/release/changesets-to-parquet \
  --input changesets-latest.osm.bz2 \
  --output-dir ./output \
  --partition \
  --generate-indexes

# 2. Upload to R2
rclone sync ./output r2:your-bucket/changesets/

# 3. On Fly.io VM:
flyctl ssh console
apt update && apt install rclone -y
rclone config  # Configure R2
rclone sync r2:your-bucket/changesets/ /data/changesets/ --progress
```

## Step 8: Test the API

```bash
# Get your app URL
flyctl info

# Test endpoints (replace with your URL)
APP_URL="https://changeset-api.fly.dev"

# Health check
curl $APP_URL/health

# Tip metadata
curl $APP_URL/meta/tip

# Coverage info
curl $APP_URL/meta/coverage

# Single changeset (replace with valid ID)
curl $APP_URL/changeset/1.json

# Recent changesets by day
curl "$APP_URL/changesets/day/20241015?limit=10"
```

## Scaling & Configuration

### Scale Memory/CPU

```bash
# Scale to 4GB RAM
flyctl scale memory 4096

# Scale to 4 CPUs
flyctl scale count 2  # Run 2 instances
```

### Update Environment Variables

```bash
flyctl secrets set DATA_PATH=/data/changesets
```

### View Metrics

```bash
flyctl dashboard  # Opens web dashboard
```

## Costs

**Estimated monthly cost:**

| Resource | Usage | Cost |
|----------|-------|------|
| VM (2 vCPU, 2GB RAM, shared) | 730 hours/mo | ~$12-15 |
| Volume (100GB) | Storage | ~$15 |
| Bandwidth | Included (free) | $0 |
| **Total** | | **~$27-30/mo** |

**Free tier:** Fly.io gives you ~$5/mo free credits, so actual cost ~$22-25/mo.

To reduce costs:
- Use smaller volume (adjust in fly.toml)
- Use 1GB RAM if sufficient
- Enable auto-stop (machines stop when idle)

## Updates & Redeployment

```bash
# Make changes to code
# Then redeploy:
flyctl deploy

# Or force rebuild:
flyctl deploy --no-cache
```

## Troubleshooting

### App won't start

```bash
# Check logs
flyctl logs

# Common issues:
# 1. DATA_PATH not set - check env vars
# 2. No data in volume - upload parquet files
# 3. Port mismatch - ensure app listens on 3000
```

### Health checks failing

```bash
# Check if /health endpoint works
flyctl ssh console
curl http://localhost:3000/health
```

### Out of memory

```bash
# Scale up memory
flyctl scale memory 4096
```

## Custom Domain (Optional)

```bash
# Add your custom domain
flyctl certs add changesets.yourdomain.com

# Point your DNS:
# CNAME: changesets.yourdomain.com -> changeset-api.fly.dev

# Fly.io will auto-provision SSL certificate
```

## Monitoring

```bash
# Real-time logs
flyctl logs --follow

# App metrics
flyctl dashboard
```

## Production Checklist

- [ ] App deployed and running
- [ ] Health endpoint returns 200 OK
- [ ] Parquet data uploaded to volume
- [ ] Test queries return valid results
- [ ] Custom domain configured (optional)
- [ ] Monitoring set up
- [ ] Cost alerts configured in Fly.io dashboard

## Next Steps

1. Test all API endpoints
2. Monitor performance and errors
3. Set up log aggregation if needed (Fly.io has built-in logs)
4. Announce to OSM community!

## Support

- Fly.io Docs: https://fly.io/docs/
- Fly.io Community: https://community.fly.io/
- API Issues: https://github.com/yourusername/changesets-to-parquet/issues

---

**You're live!** 🚀

Share your API URL with the world and start gathering feedback.
