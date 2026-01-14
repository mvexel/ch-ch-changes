#!/bin/bash
# Hetzner Cloud Ephemeral Changeset Processing
# Launches on-demand server, processes changesets, syncs back, terminates

set -e

# Parse options
FOLLOW_LOGS=false
KEEP_ON_FAILURE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --follow-logs|-f)
            FOLLOW_LOGS=true
            shift
            ;;
        --keep-on-failure|-k)
            KEEP_ON_FAILURE=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  -f, --follow-logs      Stream Docker logs during processing"
            echo "  -k, --keep-on-failure  Keep server alive for 30min on failure"
            echo "  -h, --help            Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                    # Normal run"
            echo "  $0 -f                 # Stream logs while processing"
            echo "  $0 -k                 # Keep server for debugging on failure"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Configuration
CONFIG_FILE="${CONFIG_FILE:-/opt/changeset-app/hetzner-config.env}"
if [ -f "$CONFIG_FILE" ]; then
    source "$CONFIG_FILE"
fi

HETZNER_TOKEN="${HETZNER_TOKEN}"
SERVER_TYPE="${SERVER_TYPE:-ccx33}"  # 8 vCPU, 32GB RAM
LOCATION="${LOCATION:-nbg1}"  # Nuremberg
IMAGE="${IMAGE:-ubuntu-22.04}"
SSH_KEY_NAME="${SSH_KEY_NAME:-changeset-processor}"
DEST_DATA_DIR="${DEST_DATA_DIR:-/opt/changeset-data/changesets}"
LOG_FILE="${LOG_FILE:-/var/log/changeset-hetzner-$(date +%Y%m%d-%H%M%S).log}"
REPO_DIR="${REPO_DIR:-/opt/changeset-app}"

# Create log directory
mkdir -p "$(dirname "$LOG_FILE")"

# Logging function
log() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

# Validate configuration
if [ -z "$HETZNER_TOKEN" ]; then
    echo "Error: HETZNER_TOKEN not set"
    echo "See docs/HETZNER.md for setup instructions"
    exit 1
fi

if [ ! -d "$REPO_DIR" ]; then
    echo "Error: Repository directory not found: $REPO_DIR"
    echo "Set REPO_DIR in $CONFIG_FILE or run from the repository directory"
    exit 1
fi

# Cleanup function
cleanup() {
    local exit_code=$?

    # Kill log tail if running
    if [ -n "$LOG_TAIL_PID" ]; then
        kill "$LOG_TAIL_PID" 2>/dev/null || true
    fi

    # Delete server if it exists and script is exiting with error
    if [ -n "$SERVER_ID" ] && [ $exit_code -ne 0 ]; then
        echo "Script exiting with error, deleting server $SERVER_ID..."
        curl -s -X DELETE \
            -H "Authorization: Bearer $HETZNER_TOKEN" \
            "https://api.hetzner.cloud/v1/servers/$SERVER_ID" &>/dev/null || true
        rm -f /tmp/changeset-processor-server-id
    fi
}
trap cleanup EXIT ERR INT TERM

log "=== Hetzner Changeset Processing Started ==="
log "Server Type: $SERVER_TYPE"
log "Location: $LOCATION"
log "Log file: $LOG_FILE"
echo ""

# Check if SSH key exists in Hetzner
log "[1/7] Checking SSH key..."
SSH_KEY_ID=$(curl -s -H "Authorization: Bearer $HETZNER_TOKEN" \
    "https://api.hetzner.cloud/v1/ssh_keys" | \
    jq -r ".ssh_keys[] | select(.name==\"$SSH_KEY_NAME\") | .id")

if [ -z "$SSH_KEY_ID" ] || [ "$SSH_KEY_ID" = "null" ]; then
    log "ERROR: SSH key '$SSH_KEY_NAME' not found in Hetzner"
    log "See docs/HETZNER.md for setup instructions"
    exit 1
fi

log "Using SSH key: $SSH_KEY_NAME (ID: $SSH_KEY_ID)"

# Create server
log "[2/7] Creating server..."
CREATE_RESPONSE=$(curl -s -X POST \
    -H "Authorization: Bearer $HETZNER_TOKEN" \
    -H "Content-Type: application/json" \
    -d "{
        \"name\": \"changeset-processor-$(date +%s)\",
        \"server_type\": \"$SERVER_TYPE\",
        \"location\": \"$LOCATION\",
        \"image\": \"$IMAGE\",
        \"ssh_keys\": [$SSH_KEY_ID],
        \"labels\": {
            \"purpose\": \"changeset-processing\",
            \"ephemeral\": \"true\"
        },
        \"user_data\": \"#!/bin/bash\nset -e\napt-get update\napt-get install -y docker.io\nmkdir -p /data/changesets /app\nchown ubuntu:ubuntu /app /data\"
    }" \
    "https://api.hetzner.cloud/v1/servers")

SERVER_ID=$(echo "$CREATE_RESPONSE" | jq -r '.server.id')
SERVER_IP=$(echo "$CREATE_RESPONSE" | jq -r '.server.public_net.ipv4.ip')

if [ -z "$SERVER_ID" ] || [ "$SERVER_ID" = "null" ]; then
    log "ERROR: Failed to create server"
    log "Response: $CREATE_RESPONSE"
    exit 1
fi

log "Server ID: $SERVER_ID"
log "Server IP: $SERVER_IP"
echo "$SERVER_ID" > /tmp/changeset-processor-server-id

# Wait for server to be running
log "[3/7] Waiting for server to start..."
for i in {1..60}; do
    STATUS=$(curl -s -H "Authorization: Bearer $HETZNER_TOKEN" \
        "https://api.hetzner.cloud/v1/servers/$SERVER_ID" | \
        jq -r '.server.status')

    if [ "$STATUS" = "running" ]; then
        log "Server is running!"
        break
    fi

    if [ $((i % 6)) -eq 0 ]; then
        log "Still waiting for server... (${i}/60 attempts, status: $STATUS)"
    fi
    sleep 5
done

if [ "$STATUS" != "running" ]; then
    log "ERROR: Server never started (status: $STATUS)"
    exit 1
fi

# Wait for SSH
log "[4/7] Waiting for SSH to be ready..."
SSH_READY=false
for i in {1..60}; do
    if ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o BatchMode=yes \
        root@$SERVER_IP "echo ready" &>/dev/null; then
        SSH_READY=true
        log "SSH is ready!"
        break
    fi
    if [ $((i % 6)) -eq 0 ]; then
        log "Still waiting for SSH... (${i}/60 attempts)"
    fi
    sleep 10
done

if [ "$SSH_READY" = false ]; then
    log "ERROR: SSH never became ready"
    exit 1
fi

# Wait for cloud-init to complete
log "Waiting for cloud-init to complete..."
for i in {1..60}; do
    if ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
        "cloud-init status --wait" &>/dev/null; then
        log "Cloud-init complete!"
        break
    fi
    if [ $((i % 6)) -eq 0 ]; then
        log "Still waiting for cloud-init... (${i}/60 attempts)"
    fi
    sleep 10
done

# Copy code to server
log "[5/7] Copying source code to server..."
PROCESSING_FAILED=false
if ! rsync -az --progress \
    -e "ssh -o StrictHostKeyChecking=no" \
    --exclude '.git' \
    --exclude 'target' \
    --exclude '_local' \
    "$REPO_DIR/" \
    root@$SERVER_IP:/app/ >> "$LOG_FILE" 2>&1; then
    log "ERROR: Failed to copy code to server"
    PROCESSING_FAILED=true
fi

# Build and run
if [ "$PROCESSING_FAILED" != true ]; then
    log "[6/7] Building Docker image on server..."
    if [ "$FOLLOW_LOGS" = true ]; then
        ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
            "cd /app && docker build -f Dockerfile.builder -t changeset-builder:latest ." | tee -a "$LOG_FILE"
    else
        ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
            "cd /app && docker build -f Dockerfile.builder -t changeset-builder:latest . >> /var/log/build.log 2>&1"
    fi

    if [ $? -ne 0 ]; then
        log "ERROR: Docker build failed"
        PROCESSING_FAILED=true
    fi
fi

# Start processing
if [ "$PROCESSING_FAILED" != true ]; then
    log "[7/7] Starting processing..."

    # Start the processing container in background
    ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
        "docker run --rm -v /data:/data changeset-builder:latest > /tmp/processing.log 2>&1 && touch /tmp/processing-complete" &

    # Start log tailing if requested
    if [ "$FOLLOW_LOGS" = true ]; then
        log "Streaming logs..."
        sleep 5
        ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
            "tail -f /tmp/processing.log 2>/dev/null || echo 'Waiting for logs...'; sleep infinity" &
        LOG_TAIL_PID=$!
    fi
fi

# Wait for processing
START_TIME=$(date +%s)

while [ "$PROCESSING_FAILED" != true ]; do
    if ssh -o StrictHostKeyChecking=no root@$SERVER_IP "test -f /tmp/processing-complete" 2>/dev/null; then
        DURATION=$(($(date +%s) - START_TIME))
        log "Processing complete! Duration: $((DURATION / 60)) minutes"
        break
    fi

    # Check if server is still running
    STATUS=$(curl -s -H "Authorization: Bearer $HETZNER_TOKEN" \
        "https://api.hetzner.cloud/v1/servers/$SERVER_ID" | \
        jq -r '.server.status')

    if [ "$STATUS" != "running" ]; then
        log "ERROR: Server is no longer running (status: $STATUS)"
        PROCESSING_FAILED=true
        break
    fi

    ELAPSED=$(($(date +%s) - START_TIME))
    if [ $((ELAPSED % 60)) -eq 0 ]; then
        log "Still processing... (elapsed: $((ELAPSED / 60)) minutes)"
    fi
    sleep 30
done

# Kill log tail if running
if [ -n "$LOG_TAIL_PID" ]; then
    kill "$LOG_TAIL_PID" 2>/dev/null || true
    LOG_TAIL_PID=""
fi

# Handle failure
if [ "$PROCESSING_FAILED" = true ]; then
    log "Fetching logs for debugging..."
    ssh -o StrictHostKeyChecking=no root@$SERVER_IP \
        "cat /var/log/build.log /tmp/processing.log 2>/dev/null" >> "$LOG_FILE" 2>&1 || true

    if [ "$KEEP_ON_FAILURE" = true ]; then
        log "Server kept alive for 30 minutes for debugging at: $SERVER_IP"
        log "SSH: ssh root@$SERVER_IP"
        sleep 1800
    fi

    log "Deleting server..."
    curl -s -X DELETE \
        -H "Authorization: Bearer $HETZNER_TOKEN" \
        "https://api.hetzner.cloud/v1/servers/$SERVER_ID" &>/dev/null
    log "Processing failed. Check log: $LOG_FILE"
    exit 1
fi

# Sync data back
log "[8/8] Syncing data back to local server..."
mkdir -p "$DEST_DATA_DIR"
if rsync -avz --progress \
    -e "ssh -o StrictHostKeyChecking=no" \
    root@$SERVER_IP:/data/changesets/ \
    "$DEST_DATA_DIR/" >> "$LOG_FILE" 2>&1; then
    log "Data synced successfully!"
else
    log "ERROR: rsync failed!"
    exit 1
fi

# Delete server
log "Deleting server..."
curl -s -X DELETE \
    -H "Authorization: Bearer $HETZNER_TOKEN" \
    "https://api.hetzner.cloud/v1/servers/$SERVER_ID" &>/dev/null
rm -f /tmp/changeset-processor-server-id

log ""
log "=== Complete ==="
log "Data at: $DEST_DATA_DIR"
log "Duration: $((DURATION / 60)) minutes"
log "Estimated cost: ~€$(echo "scale=2; 0.045 * $DURATION / 3600" | bc 2>/dev/null || echo 'N/A')"
log "Log file: $LOG_FILE"
log ""
log "Run 'docker compose restart api' to serve the new data."
