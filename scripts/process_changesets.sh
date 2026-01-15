#!/bin/bash
#
# Process OSM changeset archives into parquet files
# Runs on the changeset-builder Fly.io instance
#

set -e  # Exit on error
set -o pipefail  # Exit on pipe failure

# Configuration
DOWNLOAD_DIR="/data/downloads"
TEMP_OUTPUT_DIR="/data/temp_changesets"
FINAL_OUTPUT_DIR="/data/changesets"
OLD_OUTPUT_DIR="/data/changesets_old"

# OSM changeset archive URL
CHANGESET_URL="${CHANGESET_URL:-https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2}"
CHANGESET_FILE="changesets-latest.osm.bz2"

# Logging functions
log_info() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] INFO: $*"
}

log_error() {
    echo "[$(date +'%Y-%m-%d %H:%M:%S')] ERROR: $*" >&2
}

# Check volume mount and disk space
log_info "Checking disk space..."
df -h /data || log_error "/data mount not found - volume may not be mounted!"
df -h

# Create necessary directories
log_info "Creating working directories..."
mkdir -p "$DOWNLOAD_DIR" "$TEMP_OUTPUT_DIR" || {
    log_error "Failed to create directories - check volume mount"
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

# Process changesets to parquet
log_info "Processing changesets to parquet format..."
log_info "Output directory: $TEMP_OUTPUT_DIR"

START_TIME=$(date +%s)

PROCESS_CMD=(changesets-to-parquet \
    --input "$DOWNLOAD_DIR/$CHANGESET_FILE" \
    --output-dir "$TEMP_OUTPUT_DIR" \
    --partition \
    --generate-indexes \
    --batch-size 100000)

# Force line-buffered output so remote log tailing stays responsive.
if command -v stdbuf >/dev/null 2>&1; then
    PROCESS_CMD=(stdbuf -oL -eL "${PROCESS_CMD[@]}")
fi

"${PROCESS_CMD[@]}" || {
    log_error "Parquet conversion failed"
    exit 1
}

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
log_info "Parquet conversion complete. Duration: ${DURATION}s"

# Count output files
PARQUET_COUNT=$(find "$TEMP_OUTPUT_DIR" -name "*.parquet" | wc -l)
log_info "Generated $PARQUET_COUNT parquet files"

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

# Cleanup
log_info "Cleaning up..."
rm -f "$DOWNLOAD_DIR/$CHANGESET_FILE"

if [ -d "$OLD_OUTPUT_DIR" ]; then
    log_info "Removing old backup..."
    rm -rf "$OLD_OUTPUT_DIR"
fi

# Summary
log_info "Processing complete!"
log_info "Data location: $FINAL_OUTPUT_DIR"
log_info "Parquet files: $PARQUET_COUNT"
log_info "Total processing time: ${DURATION}s"

exit 0
