#!/bin/bash
#
# Process OSM changeset archives into parquet files (Native deployment version)
# Downloads, processes, and hot-swaps parquet data with zero downtime
#

set -e  # Exit on error
set -o pipefail  # Exit on pipe failure

# Configuration
DATA_ROOT="/opt/changeset-data/changesets"
DOWNLOAD_DIR="$DATA_ROOT/downloads"
TEMP_OUTPUT_DIR="$DATA_ROOT/temp_changesets"
FINAL_OUTPUT_DIR="$DATA_ROOT/parquet"
OLD_OUTPUT_DIR="$DATA_ROOT/parquet_old"

# OSM changeset archive URL
CHANGESET_URL="${CHANGESET_URL:-https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2}"
CHANGESET_FILE="changesets-latest.osm.bz2"

# Build directory (where the binary is)
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# Logging functions
log_info() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] INFO: $*"
}

log_error() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] ERROR: $*" >&2
}

# Check if running as root (needed for chown later)
if [ "$EUID" -ne 0 ]; then
    log_error "This script must be run as root (for chown permissions)"
    exit 1
fi

# Ensure cargo is available
if ! command -v cargo &> /dev/null; then
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    else
        log_error "cargo not found. Please install Rust first."
        exit 1
    fi
fi

# Check disk space
log_info "Checking disk space..."
df -h "$DATA_ROOT" || {
    log_error "$DATA_ROOT not found or not accessible"
    exit 1
}

AVAILABLE_GB=$(df -BG "$DATA_ROOT" | tail -1 | awk '{print $4}' | sed 's/G//')
if [ "$AVAILABLE_GB" -lt 50 ]; then
    log_error "Insufficient disk space. At least 50GB recommended, only ${AVAILABLE_GB}GB available"
    exit 1
fi

# Create necessary directories
log_info "Creating working directories..."
mkdir -p "$DOWNLOAD_DIR" "$TEMP_OUTPUT_DIR" || {
    log_error "Failed to create directories"
    exit 1
}

# Download changeset archive
log_info "Downloading changeset archive from $CHANGESET_URL..."
cd "$DOWNLOAD_DIR"

if [ -f "$CHANGESET_FILE" ]; then
    log_info "Existing file found, attempting to resume download..."
    wget -c "$CHANGESET_URL" -O "$CHANGESET_FILE" || {
        log_error "Download failed"
        exit 1
    }
else
    wget "$CHANGESET_URL" -O "$CHANGESET_FILE" || {
        log_error "Download failed"
        exit 1
    }
fi

FILE_SIZE=$(du -h "$CHANGESET_FILE" | cut -f1)
log_info "Download complete. File size: $FILE_SIZE"

# Build the converter if needed
log_info "Ensuring converter binary is available..."
cd "$REPO_DIR"
if [ ! -f "target/release/changesets-to-parquet" ]; then
    log_info "Building converter..."
    cargo build --release --bin changesets-to-parquet
fi

# Process changesets to parquet
log_info "Processing changesets to parquet format..."
log_info "Output directory: $TEMP_OUTPUT_DIR"

START_TIME=$(date +%s)

"$REPO_DIR/target/release/changesets-to-parquet" \
    --input "$DOWNLOAD_DIR/$CHANGESET_FILE" \
    --output-dir "$TEMP_OUTPUT_DIR" \
    --partition \
    --generate-indexes \
    --batch-size 100000 || {
    log_error "Parquet conversion failed"
    exit 1
}

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
log_info "Parquet conversion complete. Duration: ${DURATION}s ($((DURATION / 60)) minutes)"

# Count output files
PARQUET_COUNT=$(find "$TEMP_OUTPUT_DIR" -name "*.parquet" | wc -l)
log_info "Generated $PARQUET_COUNT parquet files"

# Set proper ownership before swapping
log_info "Setting ownership to changeset user..."
chown -R changeset:changeset "$TEMP_OUTPUT_DIR"

# Atomic swap: Replace old data with new data
log_info "Performing atomic swap of data directories..."

# Remove old backup if it exists
if [ -d "$OLD_OUTPUT_DIR" ]; then
    log_info "Removing previous backup..."
    rm -rf "$OLD_OUTPUT_DIR"
fi

# Move current data to backup (if exists)
if [ -d "$FINAL_OUTPUT_DIR" ]; then
    log_info "Backing up current data..."
    mv "$FINAL_OUTPUT_DIR" "$OLD_OUTPUT_DIR" || {
        log_error "Failed to backup current data"
        exit 1
    }
fi

# Move new data to final location
log_info "Moving new data to final location..."
mv "$TEMP_OUTPUT_DIR" "$FINAL_OUTPUT_DIR" || {
    log_error "Failed to move new data to final location"
    # Try to restore backup
    if [ -d "$OLD_OUTPUT_DIR" ]; then
        log_info "Attempting to restore backup..."
        mv "$OLD_OUTPUT_DIR" "$FINAL_OUTPUT_DIR"
    fi
    exit 1
}

log_info "Atomic swap complete"

# Restart the server to pick up new data
log_info "Restarting changeset-server service..."
systemctl restart changeset-server || {
    log_error "Failed to restart service"
    exit 1
}

# Wait for service to be ready
sleep 3
if systemctl is-active --quiet changeset-server; then
    log_info "Service restarted successfully"
else
    log_error "Service failed to restart. Check logs: journalctl -u changeset-server -n 50"
    exit 1
fi

# Cleanup
log_info "Cleaning up..."
rm -f "$DOWNLOAD_DIR/$CHANGESET_FILE"

if [ -d "$OLD_OUTPUT_DIR" ]; then
    log_info "Removing old backup..."
    rm -rf "$OLD_OUTPUT_DIR"
fi

# Summary
log_info "=== Processing Complete ==="
log_info "Data location: $FINAL_OUTPUT_DIR"
log_info "Parquet files: $PARQUET_COUNT"
log_info "Processing time: ${DURATION}s ($((DURATION / 60)) minutes)"
log_info "Service status: $(systemctl is-active changeset-server)"

# Test the API
if curl -sf http://localhost:3000/health > /dev/null 2>&1; then
    log_info "Health check: PASSED"
else
    log_error "Health check: FAILED"
    log_error "Check logs: journalctl -u changeset-server -f"
fi

exit 0
