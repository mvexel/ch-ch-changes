# Hetzner Cloud Ephemeral Processing Setup

Process changesets on a beefy Hetzner Cloud server, then delete it. Only pay for what you use.

## Why Hetzner?

- **10x simpler** than AWS (no IAM, security groups, VPC complexity)
- **50% cheaper** - €0.045/hour vs AWS ~$0.68/hour
- **Easy API** - Simple REST API, no complex SDK needed
- **Fast setup** - 5 minutes vs hours for AWS
- **European servers** - Good latency for OSM data

## Prerequisites

### 1. Create Hetzner Cloud Account

1. Go to https://console.hetzner.cloud/
2. Sign up for an account
3. Create a new project (e.g., "OSM Changesets")

### 2. Create API Token

1. In your project, go to **Security** → **API Tokens**
2. Click **Generate API Token**
3. Name: `changeset-processor`
4. Permissions: **Read & Write**
5. Click **Generate**
6. **Copy the token** - you won't see it again!

### 3. Create SSH Key

On your VPS, create an SSH key and upload it to Hetzner:

```bash
# Generate SSH key if you don't have one
ssh-keygen -t ed25519 -f ~/.ssh/hetzner-changeset -N ""

# Copy the public key
cat ~/.ssh/hetzner-changeset.pub
```

Now upload it to Hetzner:

1. In Hetzner Console, go to **Security** → **SSH Keys**
2. Click **Add SSH Key**
3. Name: `changeset-processor`
4. Paste the public key
5. Click **Add SSH Key**

Or via API:

```bash
# Set your token
export HETZNER_TOKEN="your_token_here"

# Upload SSH key
curl -X POST \
  -H "Authorization: Bearer $HETZNER_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"changeset-processor\",
    \"public_key\": \"$(cat ~/.ssh/hetzner-changeset.pub)\"
  }" \
  "https://api.hetzner.cloud/v1/ssh_keys"
```

### 4. Configure SSH Client

Add to `~/.ssh/config`:

```
Host *.hetzner.cloud
    User root
    IdentityFile ~/.ssh/hetzner-changeset
    StrictHostKeyChecking no
```

### 5. Install jq (JSON processor)

```bash
# Ubuntu/Debian
sudo apt-get install jq

# macOS
brew install jq
```

### 6. Configure the Script

```bash
# Copy example config
cp hetzner-config.env.example /opt/changeset-app/hetzner-config.env

# Edit with your values
nano /opt/changeset-app/hetzner-config.env
```

Set your API token:

```bash
HETZNER_TOKEN=your_token_here
SSH_KEY_NAME=changeset-processor
```

That's it! No security groups, IAM roles, or VPC configuration needed.

## Usage

### Basic Run

```bash
cd /opt/changeset-app
./scripts/hetzner-process-changesets.sh
```

### With Live Log Streaming

Stream Docker logs while processing (recommended for first run):

```bash
./scripts/hetzner-process-changesets.sh --follow-logs
```

### With Debug Mode

Keep server alive for 30 minutes on failure so you can SSH in and debug:

```bash
./scripts/hetzner-process-changesets.sh --keep-on-failure
```

### All Options

```bash
./scripts/hetzner-process-changesets.sh --help
```

**What happens:**
1. Creates on-demand Hetzner Cloud server
2. Server boots and auto-installs Docker
3. Copies your source code to the server
4. Builds Docker image on the server
5. Runs changeset processing
6. Syncs data back to VPS via rsync
7. Deletes server automatically

**Guaranteed cleanup** - Server automatically deletes even if the script crashes or is interrupted (Ctrl+C).

### Monitor Progress

The script outputs timestamped progress and logs to `/var/log/changeset-hetzner-*.log`:

```bash
# View latest log
ls -lht /var/log/changeset-hetzner-*.log | head -1

# Tail current run
tail -f /var/log/changeset-hetzner-*.log
```

### Restart API with New Data

```bash
cd /opt/changeset-app
docker compose restart api
```

## Server Types and Pricing

All prices are per hour. You typically run for 1-2 hours per week.

| Type | vCPU | RAM | Price/hour | Weekly Cost |
|------|------|-----|------------|-------------|
| ccx23 | 4 | 16GB | €0.023 | ~€0.05-0.09 |
| ccx33 | 8 | 32GB | €0.045 | ~€0.09-0.18 |
| ccx43 | 16 | 64GB | €0.090 | ~€0.18-0.36 |
| ccx53 | 32 | 128GB | €0.180 | ~€0.36-0.72 |

**Recommended:** Start with `ccx33` (8 vCPU, 32GB). It's the sweet spot for most workloads.

## Customization

### Use Different Server Type

Edit `/opt/changeset-app/hetzner-config.env`:

```bash
# For small processing
SERVER_TYPE=ccx23

# For large processing
SERVER_TYPE=ccx43
```

### Change Location

```bash
# Nuremberg, Germany (default)
LOCATION=nbg1

# Falkenstein, Germany
LOCATION=fsn1

# Helsinki, Finland
LOCATION=hel1

# Ashburn, USA
LOCATION=ash
```

### Scheduled Processing

Add to cron:

```bash
# Process weekly on Monday at 2 AM
0 2 * * 1 /opt/changeset-app/scripts/hetzner-process-changesets.sh >> /var/log/changeset-processing.log 2>&1
```

## Troubleshooting

### Check Logs

All runs are logged with timestamps:

```bash
# View latest log
ls -lht /var/log/changeset-hetzner-*.log | head -1

# Tail current run
tail -f /var/log/changeset-hetzner-*.log
```

### SSH Connection Fails

```bash
# Test SSH key manually
ssh -i ~/.ssh/hetzner-changeset root@<SERVER_IP>

# Check if SSH key exists in Hetzner
curl -H "Authorization: Bearer $HETZNER_TOKEN" \
  "https://api.hetzner.cloud/v1/ssh_keys"
```

### Processing Fails

Use `--keep-on-failure` flag to debug:

```bash
./scripts/hetzner-process-changesets.sh --keep-on-failure
```

Then SSH in while it's running:

```bash
# Get server IP from script output
ssh root@<SERVER_IP>

# Check Docker logs
docker ps -a
docker logs <container_id>

# Check build logs
cat /var/log/build.log

# Check processing logs
tail -f /tmp/processing.log
```

### Manual Cleanup

If script crashes before deleting:

```bash
# List all servers
curl -H "Authorization: Bearer $HETZNER_TOKEN" \
  "https://api.hetzner.cloud/v1/servers" | jq '.servers[] | {id, name, status}'

# Delete server manually
curl -X DELETE \
  -H "Authorization: Bearer $HETZNER_TOKEN" \
  "https://api.hetzner.cloud/v1/servers/<SERVER_ID>"
```

### View Server in Console

While processing, you can view the server in Hetzner Console:
https://console.hetzner.cloud/ → Your Project → Servers

## API Reference

The script uses Hetzner Cloud API directly. Key endpoints:

```bash
# List servers
curl -H "Authorization: Bearer $HETZNER_TOKEN" \
  "https://api.hetzner.cloud/v1/servers"

# Create server
curl -X POST \
  -H "Authorization: Bearer $HETZNER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"test","server_type":"ccx33","image":"ubuntu-22.04","ssh_keys":[123]}' \
  "https://api.hetzner.cloud/v1/servers"

# Delete server
curl -X DELETE \
  -H "Authorization: Bearer $HETZNER_TOKEN" \
  "https://api.hetzner.cloud/v1/servers/<ID>"
```

Full API docs: https://docs.hetzner.cloud/

## Cost Optimization Tips

1. **Right-size server** - Start with ccx33, scale down to ccx23 if it's enough
2. **Process incrementally** - Only process new changesets, not full history
3. **Automate scheduling** - Run weekly via cron instead of 24/7 server
4. **Monitor duration** - Script logs duration and estimated cost

## Security Notes

- Server has public IP but only accepts SSH with your key
- No persistent credentials stored on server
- Server deletes automatically after processing
- All data transfer uses SSH/rsync encryption

## Comparison to AWS

| Feature | Hetzner | AWS |
|---------|---------|-----|
| Setup complexity | 5 minutes | Hours |
| Config lines | 10 | 100+ |
| IAM required | No | Yes |
| Security groups | No | Yes |
| API complexity | Simple REST | Complex SDK |
| Cost (8 vCPU, 32GB) | €0.045/hour | ~$0.68/hour |
| Documentation | Clear | Overwhelming |

Hetzner wins on simplicity, cost, and ease of use.
