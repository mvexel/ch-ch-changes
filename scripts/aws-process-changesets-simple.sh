#!/bin/bash
# Simple on-demand instance version (faster to start, slightly more expensive)
# Good for testing or when you need results quickly

set -e

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
IMAGE_REGISTRY="${IMAGE_REGISTRY:-registry.osm.lol}"
DEST_DATA_DIR="${DEST_DATA_DIR:-/opt/changeset-data/changesets}"

if [ -z "$SECURITY_GROUP" ] || [ -z "$SUBNET_ID" ] || [ -z "$KEY_NAME" ]; then
    echo "Error: Missing configuration. See AWS_EPHEMERAL_PROCESSING.md"
    exit 1
fi

echo "=== On-Demand Changeset Processing ==="
echo "Instance Type: $INSTANCE_TYPE"
echo ""

# User data script
USER_DATA=$(cat <<'EOF'
#!/bin/bash
set -e
apt-get update
apt-get install -y docker.io
mkdir -p /data/changesets
docker pull IMAGE_REGISTRY/changeset-builder:latest
docker run --rm -v /data:/data IMAGE_REGISTRY/changeset-builder:latest
touch /tmp/processing-complete
EOF
)
USER_DATA="${USER_DATA//IMAGE_REGISTRY/$IMAGE_REGISTRY}"

# Launch on-demand instance
echo "[1/5] Launching instance..."
INSTANCE_ID=$(aws ec2 run-instances \
    --region "$AWS_REGION" \
    --image-id "$AMI_ID" \
    --instance-type "$INSTANCE_TYPE" \
    --key-name "$KEY_NAME" \
    --security-group-ids "$SECURITY_GROUP" \
    --subnet-id "$SUBNET_ID" \
    --user-data "$USER_DATA" \
    --block-device-mappings "[{\"DeviceName\":\"/dev/sda1\",\"Ebs\":{\"VolumeSize\":100,\"VolumeType\":\"gp3\",\"DeleteOnTermination\":true}}]" \
    --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=changeset-processor-ephemeral}]" \
    --instance-initiated-shutdown-behavior terminate \
    --query 'Instances[0].InstanceId' \
    --output text)

echo "Instance ID: $INSTANCE_ID"

# Wait for running
echo "[2/5] Waiting for instance to be running..."
aws ec2 wait instance-running --region "$AWS_REGION" --instance-ids "$INSTANCE_ID"

# Get public IP
PUBLIC_IP=$(aws ec2 describe-instances \
    --region "$AWS_REGION" \
    --instance-ids "$INSTANCE_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' \
    --output text)

echo "Instance running at: $PUBLIC_IP"

# Wait for SSH
echo "[3/5] Waiting for SSH..."
for i in {1..60}; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@$PUBLIC_IP "echo ready" 2>/dev/null; then
        break
    fi
    sleep 10
done

# Wait for processing
echo "[4/5] Processing (this may take a while)..."
START_TIME=$(date +%s)
while true; do
    if ssh -i "$SSH_KEY_PATH" -o StrictHostKeyChecking=no ubuntu@$PUBLIC_IP "test -f /tmp/processing-complete" 2>/dev/null; then
        DURATION=$(($(date +%s) - START_TIME))
        echo "Processing complete! Duration: $((DURATION / 60)) minutes"
        break
    fi
    echo "Still processing... ($(($(date +%s) - START_TIME)) seconds elapsed)"
    sleep 30
done

# Sync data
echo "[5/5] Syncing data..."
mkdir -p "$DEST_DATA_DIR"
rsync -avz --progress \
    -e "ssh -i $SSH_KEY_PATH -o StrictHostKeyChecking=no" \
    ubuntu@$PUBLIC_IP:/data/changesets/ \
    "$DEST_DATA_DIR/"

# Terminate
echo "Terminating instance..."
aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID"

echo ""
echo "=== Complete ==="
echo "Data at: $DEST_DATA_DIR"
echo "Duration: $((DURATION / 60)) minutes"
echo ""
echo "Run 'docker compose restart api' to serve the new data."
