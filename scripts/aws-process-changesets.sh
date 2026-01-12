#!/bin/bash
# Ephemeral AWS processing orchestrator
# Run this on your VPS to spin up AWS instance, process changesets, sync back, and terminate

set -e

# Configuration
CONFIG_FILE="${CONFIG_FILE:-/opt/changeset-app/aws-config.env}"
if [ -f "$CONFIG_FILE" ]; then
    source "$CONFIG_FILE"
fi

# Required settings
AWS_REGION="${AWS_REGION:-us-east-1}"
INSTANCE_TYPE="${INSTANCE_TYPE:-c6i.4xlarge}"  # 16 vCPU, 32GB RAM
AMI_ID="${AMI_ID:-ami-0c55b159cbfafe1f0}"  # Ubuntu 22.04 in us-east-1
SECURITY_GROUP="${SECURITY_GROUP}"
SUBNET_ID="${SUBNET_ID}"
KEY_NAME="${KEY_NAME}"
SSH_KEY_PATH="${SSH_KEY_PATH:-~/.ssh/aws-changeset-processor.pem}"
SPOT_PRICE="${SPOT_PRICE:-0.50}"  # Max price per hour
IMAGE_REGISTRY="${IMAGE_REGISTRY:-registry.osm.lol}"
DEST_DATA_DIR="${DEST_DATA_DIR:-/opt/changeset-data/changesets}"

# Validate required settings
if [ -z "$SECURITY_GROUP" ] || [ -z "$SUBNET_ID" ] || [ -z "$KEY_NAME" ]; then
    echo "Error: Missing required configuration."
    echo "Create $CONFIG_FILE with:"
    echo "  SECURITY_GROUP=sg-xxxxx"
    echo "  SUBNET_ID=subnet-xxxxx"
    echo "  KEY_NAME=your-key-name"
    exit 1
fi

echo "=== Ephemeral Changeset Processing ==="
echo "Region: $AWS_REGION"
echo "Instance Type: $INSTANCE_TYPE"
echo "Max Spot Price: \$$SPOT_PRICE/hour"
echo ""

# Create user data script
USER_DATA=$(cat <<'EOF'
#!/bin/bash
set -e

# Install Docker
apt-get update
apt-get install -y docker.io

# Create data directory
mkdir -p /data/changesets

# Pull and run processing container
docker pull IMAGE_REGISTRY/changeset-builder:latest
docker run --rm -v /data:/data IMAGE_REGISTRY/changeset-builder:latest

# Signal completion
touch /tmp/processing-complete
echo "Processing complete at $(date)" > /tmp/processing-status
EOF
)

# Replace placeholder with actual registry
USER_DATA="${USER_DATA//IMAGE_REGISTRY/$IMAGE_REGISTRY}"

# Launch spot instance
echo "[1/6] Launching spot instance..."
SPOT_REQUEST=$(aws ec2 request-spot-instances \
    --region "$AWS_REGION" \
    --spot-price "$SPOT_PRICE" \
    --instance-count 1 \
    --type "one-time" \
    --launch-specification "{
        \"ImageId\": \"$AMI_ID\",
        \"InstanceType\": \"$INSTANCE_TYPE\",
        \"KeyName\": \"$KEY_NAME\",
        \"SecurityGroupIds\": [\"$SECURITY_GROUP\"],
        \"SubnetId\": \"$SUBNET_ID\",
        \"UserData\": \"$(echo "$USER_DATA" | base64 -w 0)\",
        \"BlockDeviceMappings\": [{
            \"DeviceName\": \"/dev/sda1\",
            \"Ebs\": {
                \"VolumeSize\": 100,
                \"VolumeType\": \"gp3\",
                \"DeleteOnTermination\": true
            }
        }]
    }" \
    --output text \
    --query 'SpotInstanceRequests[0].SpotInstanceRequestId')

echo "Spot request ID: $SPOT_REQUEST"

# Wait for spot request to be fulfilled
echo "[2/6] Waiting for spot request to be fulfilled..."
aws ec2 wait spot-instance-request-fulfilled \
    --region "$AWS_REGION" \
    --spot-instance-request-ids "$SPOT_REQUEST"

# Get instance ID
INSTANCE_ID=$(aws ec2 describe-spot-instance-requests \
    --region "$AWS_REGION" \
    --spot-instance-request-ids "$SPOT_REQUEST" \
    --output text \
    --query 'SpotInstanceRequests[0].InstanceId')

echo "Instance ID: $INSTANCE_ID"

# Tag instance for easy identification
aws ec2 create-tags \
    --region "$AWS_REGION" \
    --resources "$INSTANCE_ID" \
    --tags "Key=Name,Value=changeset-processor-ephemeral" "Key=Purpose,Value=changeset-processing"

# Wait for instance to be running
echo "[3/6] Waiting for instance to be running..."
aws ec2 wait instance-running \
    --region "$AWS_REGION" \
    --instance-ids "$INSTANCE_ID"

# Get public IP
PUBLIC_IP=$(aws ec2 describe-instances \
    --region "$AWS_REGION" \
    --instance-ids "$INSTANCE_ID" \
    --output text \
    --query 'Reservations[0].Instances[0].PublicIpAddress')

echo "Instance running at: $PUBLIC_IP"

# Wait for SSH to be available
echo "[4/6] Waiting for SSH to be ready..."
for i in {1..60}; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@$PUBLIC_IP "echo SSH ready" 2>/dev/null; then
        echo "SSH is ready!"
        break
    fi
    echo "Attempt $i/60: SSH not ready yet, waiting..."
    sleep 10
done

# Wait for processing to complete
echo "[5/6] Waiting for processing to complete..."
echo "This may take a while depending on the data size..."
START_TIME=$(date +%s)

while true; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP "test -f /tmp/processing-complete" 2>/dev/null; then
        END_TIME=$(date +%s)
        DURATION=$((END_TIME - START_TIME))
        echo "Processing complete! Duration: $((DURATION / 60)) minutes"
        break
    fi

    ELAPSED=$(($(date +%s) - START_TIME))
    echo "Still processing... (elapsed: $((ELAPSED / 60)) minutes)"
    sleep 30
done

# Sync data back to VPS
echo "[6/6] Syncing data back to VPS..."
mkdir -p "$DEST_DATA_DIR"
rsync -avz --progress \
    -e "ssh -i $SSH_KEY_PATH -o StrictHostKeyChecking=no" \
    ubuntu@$PUBLIC_IP:/data/changesets/ \
    "$DEST_DATA_DIR/"

echo "Data synced successfully!"

# Terminate instance
echo "Terminating instance..."
aws ec2 terminate-instances \
    --region "$AWS_REGION" \
    --instance-ids "$INSTANCE_ID"

# Cancel spot request
aws ec2 cancel-spot-instance-requests \
    --region "$AWS_REGION" \
    --spot-instance-request-ids "$SPOT_REQUEST"

echo ""
echo "=== Complete ==="
echo "Instance terminated: $INSTANCE_ID"
echo "Data available at: $DEST_DATA_DIR"
echo "Total duration: $((DURATION / 60)) minutes"
echo "Estimated cost: ~\$$(echo "scale=2; $SPOT_PRICE * $DURATION / 3600" | bc)"
echo ""
echo "Run 'docker compose restart api' to serve the new data."
