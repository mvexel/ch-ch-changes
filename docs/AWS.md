# AWS Ephemeral Processing Setup

Process changesets on a beefy EC2 instance, then terminate it. Only pay for what you use.

## Overview

Process changesets on AWS EC2 and upload to S3 for efficient DuckDB querying with glob patterns. Also copies the parquet files back to local FS for API access.

## Prerequisites

### 1. Create IAM User 

You need an IAM user with permissions to manage EC2 instances.

**Via AWS Console:**

1. Sign in to AWS Console → Search for "IAM"
2. Click **Users** → **Create user**
3. Username: `changeset-processor` → **Next**
4. Select **Attach policies directly** → **Create policy**
5. Click **JSON** tab and paste:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "ec2:RunInstances",
        "ec2:TerminateInstances",
        "ec2:DescribeInstances",
        "ec2:CreateTags",
        "ec2:CreateKeyPair",
        "ec2:DescribeKeyPairs",
        "ec2:DeleteKeyPair",
        "ec2:CreateSecurityGroup",
        "ec2:AuthorizeSecurityGroupIngress",
        "ec2:DescribeSecurityGroups",
        "ec2:DescribeVpcs",
        "ec2:DescribeSubnets",
        "ec2:GetConsoleOutput"
      ],
      "Resource": "*"
    }
  ]
}
```

6. Click **Next** → Name: `ChangesetProcessorPolicy` → **Create policy**
7. Go back to user creation tab → Refresh → Select your new policy → **Next** → **Create user**
8. Click the user → **Security credentials** → **Create access key**
9. Select **Command Line Interface (CLI)** → Confirm → **Create**
10. **Save both keys!** (Access Key ID and Secret Access Key) - Download CSV as backup

### 2. Install AWS CLI on VPS

SSH to your VPS:

```bash
# Install AWS CLI
curl "https://awscli.amazonaws.com/awscli-exe-linux-x86_64.zip" -o "awscliv2.zip"
unzip awscliv2.zip
sudo ./aws/install

# Verify
aws --version
```

### 3. Configure AWS Credentials on VPS

```bash
# Configure with the access keys you saved
aws configure

# Enter:
# - AWS Access Key ID: [paste from step 1]
# - AWS Secret Access Key: [paste from step 1]
# - Default region: us-east-1
# - Default output format: json
```

**Test it works:**
```bash
aws ec2 describe-instances --output table
```

(should return empty table)

### 4. Create SSH Key Pair

Now that credentials are configured, create the SSH key:

```bash
# Create .ssh directory if it doesn't exist
mkdir -p ~/.ssh

# Create the key pair via AWS
aws ec2 create-key-pair \
  --key-name changeset-processor \
  --query 'KeyMaterial' \
  --output text > ~/.ssh/aws-processor.pem

chmod 600 ~/.ssh/aws-processor.pem

# Verify it was created
aws ec2 describe-key-pairs --key-names changeset-processor
```

### 5. Create Security Group

```bash
# Get your VPS public IP
VPS_IP=$(curl -s ifconfig.me)

# Create security group
SG_ID=$(aws ec2 create-security-group \
  --group-name changeset-processor-sg \
  --description "Security group for ephemeral changeset processor" \
  --query 'GroupId' \
  --output text)

# Allow SSH from your VPS only
aws ec2 authorize-security-group-ingress \
  --group-id $SG_ID \
  --protocol tcp \
  --port 22 \
  --cidr $VPS_IP/32

echo "Security Group ID: $SG_ID"
# Save this if you want to pin the ID; otherwise the script can auto-discover by name
```
### 6. Get Subnet ID

```bash
# List your VPCs
aws ec2 describe-vpcs

# List subnets in default VPC
aws ec2 describe-subnets \
  --filters "Name=default-for-az,Values=true" \
  --query 'Subnets[0].SubnetId' \
  --output text
# Save this if you want to pin the subnet; otherwise the script can auto-discover
```

### 7. Create S3 Bucket and IAM Role

Create a public S3 bucket for hosting the parquet files:

```bash
# Create bucket
aws s3 mb s3://osm-changesets-parquet --region us-east-1

# Enable public read access (for public data)
aws s3api put-public-access-block \
    --bucket osm-changesets-parquet \
    --public-access-block-configuration \
    "BlockPublicAcls=false,IgnorePublicAcls=false,BlockPublicPolicy=false,RestrictPublicBuckets=false"

# Set bucket policy
cat > /tmp/bucket-policy.json <<EOF
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "PublicReadGetObject",
      "Effect": "Allow",
      "Principal": "*",
      "Action": ["s3:GetObject", "s3:ListBucket"],
      "Resource": [
        "arn:aws:s3:::osm-changesets-parquet",
        "arn:aws:s3:::osm-changesets-parquet/*"
      ]
    }
  ]
}
EOF

aws s3api put-bucket-policy \
    --bucket osm-changesets-parquet \
    --policy file:///tmp/bucket-policy.json
```

Create IAM role for EC2 to upload to S3:

```bash
# Create trust policy
cat > /tmp/trust-policy.json <<EOF
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {"Service": "ec2.amazonaws.com"},
      "Action": "sts:AssumeRole"
    }
  ]
}
EOF

# Create role
aws iam create-role \
    --role-name changeset-processor-role \
    --assume-role-policy-document file:///tmp/trust-policy.json

# Create S3 access policy
cat > /tmp/s3-policy.json <<EOF
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:DeleteObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::osm-changesets-parquet",
        "arn:aws:s3:::osm-changesets-parquet/*"
      ]
    }
  ]
}
EOF

# Attach policy
aws iam put-role-policy \
    --role-name changeset-processor-role \
    --policy-name S3Access \
    --policy-document file:///tmp/s3-policy.json

# Create instance profile
aws iam create-instance-profile \
    --instance-profile-name changeset-processor-profile

# Add role to profile
aws iam add-role-to-instance-profile \
    --instance-profile-name changeset-processor-profile \
    --role-name changeset-processor-role

echo "Instance profile: changeset-processor-profile"
# Save this - you'll need it in step 8
```

### 8. Configure the Script

```bash
# Copy example config
cp aws-config.env.example /opt/changeset-app/aws-config.env

# Edit with your values
nano /opt/changeset-app/aws-config.env
```

Set these values (from previous steps). If you used the default names,
you can leave `SECURITY_GROUP` and `SUBNET_ID` empty and the script will
auto-discover them.
```bash
# Core settings
SECURITY_GROUP=sg-xxxxx                          # From step 5 (optional)
SECURITY_GROUP_NAME=changeset-processor-sg        # Default group name
SUBNET_ID=subnet-xxxxx                           # From step 6 (optional)
KEY_NAME=changeset-processor                     # From step 4 (default name)
SSH_KEY_PATH=~/.ssh/aws-processor.pem           # From step 4

# S3 Upload (required for DuckDB glob queries)
S3_BUCKET=s3://osm-changesets-parquet           # From step 7
IAM_INSTANCE_PROFILE=changeset-processor-profile # From step 7
```

## Usage

### Basic Run

```bash
cd /opt/changeset-app
./scripts/aws-process-changesets.sh
```

### With Live Log Streaming

Stream Docker logs while processing (recommended for first run):

```bash
./scripts/aws-process-changesets.sh --follow-logs
```

### With Debug Mode

Keep instance alive for 30 minutes on failure so you can SSH in and debug:

```bash
./scripts/aws-process-changesets.sh --keep-on-failure
```

### All Options

```bash
./scripts/aws-process-changesets.sh --help
```

**What happens:**
1. Launches on-demand EC2 instance with IAM role
2. Instance boots and auto-installs Docker and AWS CLI
3. Copies your source code to the instance
4. Builds Docker image on the instance
5. Runs changeset processing
6. Uploads data to S3 (public bucket for DuckDB queries)
7. Syncs data back to VPS via rsync
8. Terminates instance automatically

**Guaranteed cleanup** - Instance automatically terminates even if the script crashes or is interrupted (Ctrl+C).

### Monitor Progress

The script outputs timestamped progress and logs to `/var/log/changeset-aws-*.log`:

### Restart API with New Data

```bash
cd /opt/changeset-app
docker compose restart api
```

## Customization

### Process Specific Date Range

Edit the user data in the script to pass environment variables:

```bash
# In aws-process-changesets.sh, modify the docker run line:
docker run --rm -v /data:/data \
  -e START_DATE=2024-01-01 \
  -e END_DATE=2024-06-30 \
  IMAGE_REGISTRY/changeset-builder:latest
```

### Use Different Instance Type

Edit `/opt/changeset-app/aws-config.env`:

```bash
# For small processing
INSTANCE_TYPE=c6i.2xlarge  # 8 vCPU, 16GB

# For large processing
INSTANCE_TYPE=c6i.8xlarge  # 32 vCPU, 64GB
```

### Scheduled Processing

Add to cron:

```bash
# Process monthly on 1st day at 2 AM
0 2 1 * * /opt/changeset-app/scripts/aws-process-changesets.sh >> /var/log/changeset-processing.log 2>&1
```

## Troubleshooting

### Check Logs

All runs are logged with timestamps:

```bash
# View latest log
ls -lht /var/log/changeset-aws-*.log | head -1

# Tail current run
tail -f /var/log/changeset-aws-*.log
```

### SSH connection fails

```bash
# Check security group allows your IP
aws ec2 describe-security-groups --group-ids sg-xxxxx

# Verify key permissions
chmod 600 ~/.ssh/aws-processor.pem

# Test SSH manually (use IP from script output)
ssh -i ~/.ssh/aws-processor.pem ubuntu@<INSTANCE_IP>
```

### Processing fails

Use `--keep-on-failure` flag to debug:

```bash
./scripts/aws-process-changesets.sh --keep-on-failure
```

Or SSH in manually while it's running:

```bash
# Get instance IP from script output
ssh -i ~/.ssh/aws-processor.pem ubuntu@<INSTANCE_IP>

# Check Docker logs
sudo docker ps -a
sudo docker logs <container_id>

# Check user data execution
sudo tail -f /var/log/cloud-init-output.log
```

### Manual cleanup

If script crashes before terminating:

```bash
# Check for orphaned instances
aws ec2 describe-instances \
  --filters "Name=tag:Name,Values=changeset-processor-ephemeral" "Name=instance-state-name,Values=running" \
  --query 'Reservations[].Instances[].[InstanceId,LaunchTime,State.Name]' \
  --output table

# Terminate manually if needed
aws ec2 terminate-instances --instance-ids i-xxxxx
```

## Cost Optimization Tips

1. **Right-size instance** - Start with c6i.2xlarge, scale up if needed
2. **Process incrementally** - Only process new changesets, not full history
3. **Use gp3 volumes** - Already configured (cheaper and faster than gp2)
4. **Automate scheduling** - Run weekly via cron instead of 24/7 server

## Security Notes

- Security group only allows SSH from your VPS IP
- Instance has no public services exposed
- Data transfer uses SSH/rsync encryption
- Instance terminates automatically after processing
- No persistent credentials stored on instance
