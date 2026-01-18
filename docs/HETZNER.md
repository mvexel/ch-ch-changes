# Hetzner 

## Setup

copy example config

```bash
cp hetzner-config.env.example /opt/changeset-app/hetzner-config.env
```

Set API token in .env (get from cloud console)

```bash
HETZNER_TOKEN=your_token_here
SSH_KEY_NAME=changeset-processor
```

## Usage

One off

```bash
cd /opt/changeset-app
./scripts/hetzner-process-changesets.sh
```

Or with log tail

```bash
./scripts/hetzner-process-changesets.sh --follow-logs
```

Or keep server alive for 30 minutes to debug

```bash
./scripts/hetzner-process-changesets.sh --keep-on-failure
```

The script outputs timestamped progress and logs to `/var/log/changeset-hetzner-*.log`

### Restart API with New Data

The script automatically restarts the changeset-server systemd service after syncing new data. If you need to restart manually:

```bash
systemctl restart changeset-server
systemctl status changeset-server
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

# Check processing logs
tail -f /tmp/processing.log

# Check if binary exists and is executable
ls -la /usr/local/bin/changesets-to-parquet

# Test the processing script
cd /app
./process_changesets.sh
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
