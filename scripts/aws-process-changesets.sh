#!/bin/bash
# AWS Ephemeral Changeset Processing
# Launches on-demand EC2 instance, processes changesets, syncs back, terminates

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
            echo "  -k, --keep-on-failure  Keep instance alive for 30min on failure"
            echo "  -h, --help            Show this help"
            echo ""
            echo "Examples:"
            echo "  $0                    # Normal run"
            echo "  $0 -f                 # Stream logs while processing"
            echo "  $0 -k                 # Keep instance for debugging on failure"
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
CONFIG_FILE="${CONFIG_FILE:-/opt/changeset-app/aws-config.env}"
if [ -f "$CONFIG_FILE" ]; then
    source "$CONFIG_FILE"
fi

AWS_REGION="${AWS_REGION:-us-east-1}"
INSTANCE_TYPE="${INSTANCE_TYPE:-c6i.4xlarge}"
AMI_ID="${AMI_ID:-ami-0c55b159cbfafe1f0}"
SECURITY_GROUP="${SECURITY_GROUP}"
SUBNET_ID="${SUBNET_ID}"
KEY_NAME="${KEY_NAME}"
SSH_KEY_PATH="${SSH_KEY_PATH:-~/.ssh/aws-changeset-processor.pem}"
DEST_DATA_DIR="${DEST_DATA_DIR:-/opt/changeset-data/changesets}"
LOG_FILE="${LOG_FILE:-/var/log/changeset-aws-$(date +%Y%m%d-%H%M%S).log}"
REPO_DIR="${REPO_DIR:-/opt/changeset-app}"

# Validate configuration
if [ -z "$SECURITY_GROUP" ] || [ -z "$SUBNET_ID" ] || [ -z "$KEY_NAME" ]; then
    echo "Error: Missing configuration. See AWS_EPHEMERAL_PROCESSING.md"
    exit 1
fi

# Validate repo directory exists
if [ ! -d "$REPO_DIR" ]; then
    echo "Error: Repository directory not found: $REPO_DIR"
    echo "Set REPO_DIR in $CONFIG_FILE or run from the repository directory"
    exit 1
fi

# Create log directory
mkdir -p "$(dirname "$LOG_FILE")"

# Logging function
log() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG_FILE"
}

# Cleanup function - ensures instance is terminated even on unexpected error
cleanup() {
    local exit_code=$?

    # Kill log tail if running
    if [ -n "$LOG_TAIL_PID" ]; then
        kill "$LOG_TAIL_PID" 2>/dev/null || true
    fi

    # Terminate instance if it exists and script is exiting with error
    if [ -n "$INSTANCE_ID" ] && [ $exit_code -ne 0 ]; then
        echo "Script exiting with error, terminating instance $INSTANCE_ID..."
        aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" &>/dev/null || true
        rm -f /tmp/changeset-processor-instance-id
    fi
}
trap cleanup EXIT ERR INT TERM

log "=== AWS Changeset Processing Started ==="
log "Instance Type: $INSTANCE_TYPE"
log "Region: $AWS_REGION"
log "Log file: $LOG_FILE"
echo ""

# User data script - just install Docker
USER_DATA=$(cat <<'EOF'
#!/bin/bash
set -e
exec > >(tee /var/log/user-data.log)
exec 2>&1

echo "[$(date)] Starting user-data script..."

echo "[$(date)] Updating packages..."
apt-get update

echo "[$(date)] Installing Docker..."
apt-get install -y docker.io

echo "[$(date)] Creating directories..."
mkdir -p /data/changesets /app
chown ubuntu:ubuntu /app /data

echo "[$(date)] Docker installation complete, ready for builds"
EOF
)

# Launch instance
log "[1/7] Launching instance..."
INSTANCE_ID=$(aws ec2 run-instances \
    --region "$AWS_REGION" \
    --image-id "$AMI_ID" \
    --instance-type "$INSTANCE_TYPE" \
    --key-name "$KEY_NAME" \
    --security-group-ids "$SECURITY_GROUP" \
    --subnet-id "$SUBNET_ID" \
    --user-data "$USER_DATA" \
    --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":100,\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
    --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=changeset-processor-ephemeral},{Key=LaunchedBy,Value=aws-process-changesets}]" \
    --instance-initiated-shutdown-behavior terminate \
    --query 'Instances[0].InstanceId' \
    --output text 2>> "$LOG_FILE")

log "Instance ID: $INSTANCE_ID"
echo "$INSTANCE_ID" > /tmp/changeset-processor-instance-id

# Wait for running
log "[2/7] Waiting for instance to be running..."
aws ec2 wait instance-running --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" 2>> "$LOG_FILE"

# Get public IP
PUBLIC_IP=$(aws ec2 describe-instances \
    --region "$AWS_REGION" \
    --instance-ids "$INSTANCE_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' \
    --output text)

log "Instance running at: $PUBLIC_IP"
log "SSH command: ssh -i $SSH_KEY_PATH ubuntu@$PUBLIC_IP"
echo ""

# Wait for SSH
log "[3/7] Waiting for SSH to be ready..."
SSH_READY=false
for i in {1..60}; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@$PUBLIC_IP "echo ready" &>/dev/null; then
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
    log "Fetching console output for debugging..."
    aws ec2 get-console-output --region "$AWS_REGION" --instance-id "$INSTANCE_ID" --output text >> "$LOG_FILE" 2>&1 || true
    log "Check log file: $LOG_FILE"

    if [ "$KEEP_ON_FAILURE" = true ]; then
        log "Instance kept alive for 30 minutes for debugging"
        sleep 1800
    fi

    aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" &>/dev/null
    exit 1
fi

# Wait for user-data to complete
log "Waiting for user-data to complete..."
for i in {1..60}; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP "cloud-init status --wait" &>/dev/null; then
        log "User-data complete!"
        break
    fi
    if [ $((i % 6)) -eq 0 ]; then
        log "Still waiting for user-data... (${i}/60 attempts)"
    fi
    sleep 10
done

# Verify /app exists and is writable
if ! ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP "test -w /app" &>/dev/null; then
    log "ERROR: /app directory not ready"
    PROCESSING_FAILED=true
fi

if [ "$PROCESSING_FAILED" = true ]; then
    log "Fetching logs for debugging..."
    ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
        "sudo cat /var/log/user-data.log" >> "$LOG_FILE" 2>&1 || true

    if [ "$KEEP_ON_FAILURE" = true ]; then
        log "Instance kept alive for 30 minutes for debugging at: $PUBLIC_IP"
        log "SSH: ssh -i $SSH_KEY_PATH ubuntu@$PUBLIC_IP"
        sleep 1800
    fi

    aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" &>/dev/null
    exit 1
fi

# Copy code to instance
log "[4/7] Copying source code to instance..."
if ! rsync -az --progress \
    -e "ssh -i $SSH_KEY_PATH -o StrictHostKeyChecking=no" \
    --exclude '.git' \
    --exclude 'target' \
    --exclude '_local' \
    "$REPO_DIR/" \
    ubuntu@$PUBLIC_IP:/app/ >> "$LOG_FILE" 2>&1; then
    log "ERROR: Failed to copy code to instance"
    PROCESSING_FAILED=true
fi

if [ "$PROCESSING_FAILED" != true ]; then
    # Build Docker image on instance
    log "[5/7] Building Docker image on instance..."
    if [ "$FOLLOW_LOGS" = true ]; then
        ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
            "cd /app && sudo docker build -f Dockerfile.builder -t changeset-builder:latest . 2>&1 | tee -a /var/log/user-data.log" | tee -a "$LOG_FILE"
    else
        ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
            "cd /app && sudo docker build -f Dockerfile.builder -t changeset-builder:latest . >> /var/log/user-data.log 2>&1"
    fi

    if [ $? -ne 0 ]; then
        log "ERROR: Docker build failed"
        PROCESSING_FAILED=true
    fi
fi

# Start processing if build succeeded
if [ "$PROCESSING_FAILED" != true ]; then
    log "[6/7] Starting processing..."

    # Start the processing container in background
    ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
        "sudo docker run --rm -v /data:/data changeset-builder:latest > /var/log/processing.log 2>&1 && touch /tmp/processing-complete" &

    # Start log tailing if requested
    if [ "$FOLLOW_LOGS" = true ]; then
        log "Streaming logs..."
        sleep 5  # Give container time to start
        ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
            "tail -f /var/log/processing.log 2>/dev/null || echo 'Waiting for logs...'; sleep infinity" &
        LOG_TAIL_PID=$!
    fi
fi

# Wait for processing
START_TIME=$(date +%s)

while [ "$PROCESSING_FAILED" != true ]; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP "test -f /tmp/processing-complete" 2>/dev/null; then
        DURATION=$(($(date +%s) - START_TIME))
        log "Processing complete! Duration: $((DURATION / 60)) minutes"
        break
    fi

    # Check if instance is still running
    STATE=$(aws ec2 describe-instances \
        --region "$AWS_REGION" \
        --instance-ids "$INSTANCE_ID" \
        --query 'Reservations[0].Instances[0].State.Name' \
        --output text)

    if [ "$STATE" != "running" ]; then
        log "ERROR: Instance is no longer running (state: $STATE)"
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
    ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP \
        "sudo cat /var/log/user-data.log" >> "$LOG_FILE" 2>&1 || true

    if [ "$KEEP_ON_FAILURE" = true ]; then
        log "Instance kept alive for 30 minutes for debugging at: $PUBLIC_IP"
        log "SSH: ssh -i $SSH_KEY_PATH ubuntu@$PUBLIC_IP"
        sleep 1800
    fi

    log "Terminating instance..."
    aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" &>/dev/null
    log "Processing failed. Check log: $LOG_FILE"
    exit 1
fi

# Sync data
log "[7/7] Syncing data back to VPS..."
mkdir -p "$DEST_DATA_DIR"
if rsync -avz --progress \
    -e "ssh -i $SSH_KEY_PATH -o StrictHostKeyChecking=no" \
    ubuntu@$PUBLIC_IP:/data/changesets/ \
    "$DEST_DATA_DIR/" >> "$LOG_FILE" 2>&1; then
    log "Data synced successfully!"
else
    log "ERROR: rsync failed!"
    exit 1
fi

# Terminate
log "[6/6] Terminating instance..."
aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" &>/dev/null
rm -f /tmp/changeset-processor-instance-id

log ""
log "=== Complete ==="
log "Data at: $DEST_DATA_DIR"
log "Duration: $((DURATION / 60)) minutes"
log "Estimated cost: ~\$$(echo "scale=2; 0.68 * $DURATION / 3600" | bc 2>/dev/null || echo 'N/A')"
log "Log file: $LOG_FILE"
log ""
log "Run 'docker compose restart api' to serve the new data."
