#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
DEPLOY_DIR="${DEPLOY_DIR:-/opt/chch}"
DATA_DIR="${DATA_DIR:-$DEPLOY_DIR/data}"
DOWNLOAD_DIR="${DOWNLOAD_DIR:-/tmp/chch-init}"
PLANET_URL="${PLANET_URL:-https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2}"
CONVERTER_BIN="${CONVERTER_BIN:-$DEPLOY_DIR/bin/chch-convert}"

# Parse arguments
SKIP_DOWNLOAD=false
INPUT_FILE=""
KEEP_DOWNLOAD=false

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Initialize ch-ch-changes data by downloading and converting OSM planet changeset dump.

Options:
    -i, --input FILE        Use existing changeset file instead of downloading
    -s, --skip-download     Skip download if file already exists in $DOWNLOAD_DIR
    -k, --keep              Keep downloaded file after conversion
    -d, --data-dir DIR      Output directory for parquet files (default: $DATA_DIR)
    -u, --url URL           Planet dump URL (default: OSM changesets-latest)
    -h, --help              Show this help message

Examples:
    # Download and convert latest planet dump
    $0

    # Use existing file
    $0 --input /path/to/changesets.osm.bz2

    # Skip download if file exists, keep it after
    $0 --skip-download --keep

Environment Variables:
    DEPLOY_DIR       Base deployment directory (default: /opt/chch)
    DATA_DIR         Output directory for parquet files (default: \$DEPLOY_DIR/data)
    DOWNLOAD_DIR     Temporary download directory (default: /tmp/chch-init)
    PLANET_URL       URL to download from (default: OSM changesets-latest)
    CONVERTER_BIN    Path to converter binary (default: \$DEPLOY_DIR/bin/chch-convert)
EOF
    exit 1
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -i|--input)
            INPUT_FILE="$2"
            shift 2
            ;;
        -s|--skip-download)
            SKIP_DOWNLOAD=true
            shift
            ;;
        -k|--keep)
            KEEP_DOWNLOAD=true
            shift
            ;;
        -d|--data-dir)
            DATA_DIR="$2"
            shift 2
            ;;
        -u|--url)
            PLANET_URL="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            usage
            ;;
    esac
done

echo -e "${BLUE}=== ch-ch-changes Initial Data Load ===${NC}"
echo ""

# Check converter exists
if [ ! -x "$CONVERTER_BIN" ]; then
    echo -e "${RED}Error: Converter not found at $CONVERTER_BIN${NC}"
    echo "Please run setup.sh and deploy.sh first to build the converter"
    exit 1
fi

echo "Configuration:"
echo "  Converter:    $CONVERTER_BIN"
echo "  Data dir:     $DATA_DIR"
echo "  Download dir: $DOWNLOAD_DIR"
echo ""

# Determine input file
if [ -n "$INPUT_FILE" ]; then
    if [ ! -f "$INPUT_FILE" ]; then
        echo -e "${RED}Error: Input file not found: $INPUT_FILE${NC}"
        exit 1
    fi
    echo -e "${GREEN}Using provided input file: $INPUT_FILE${NC}"
    CHANGESET_FILE="$INPUT_FILE"
else
    # Download if needed
    mkdir -p "$DOWNLOAD_DIR"
    CHANGESET_FILE="$DOWNLOAD_DIR/$(basename "$PLANET_URL")"

    if [ -f "$CHANGESET_FILE" ] && [ "$SKIP_DOWNLOAD" = true ]; then
        echo -e "${YELLOW}Found existing file, skipping download: $CHANGESET_FILE${NC}"
        FILE_SIZE=$(du -h "$CHANGESET_FILE" | cut -f1)
        echo "  Size: $FILE_SIZE"
    else
        echo -e "${YELLOW}[1/3] Downloading planet changeset dump...${NC}"
        echo "  URL: $PLANET_URL"
        echo "  Destination: $CHANGESET_FILE"
        echo ""

        # Download with progress and resume support
        if command -v wget &> /dev/null; then
            wget -c -O "$CHANGESET_FILE" "$PLANET_URL"
        elif command -v curl &> /dev/null; then
            curl -C - -L -o "$CHANGESET_FILE" "$PLANET_URL"
        else
            echo -e "${RED}Error: Neither wget nor curl found${NC}"
            exit 1
        fi

        if [ ! -f "$CHANGESET_FILE" ]; then
            echo -e "${RED}Error: Download failed${NC}"
            exit 1
        fi

        FILE_SIZE=$(du -h "$CHANGESET_FILE" | cut -f1)
        echo -e "${GREEN}✓ Download complete (${FILE_SIZE})${NC}"
    fi
fi

# Create output directory
echo -e "\n${YELLOW}[2/3] Preparing output directory...${NC}"
mkdir -p "$DATA_DIR"

# Check permissions
if [ ! -w "$DATA_DIR" ]; then
    echo -e "${YELLOW}Warning: Cannot write to $DATA_DIR${NC}"
    echo "You may need to run this script with sudo, or adjust permissions"
fi

# Convert the data
echo -e "\n${YELLOW}[3/3] Converting changeset data to Parquet...${NC}"
echo "This may take a while depending on file size..."
echo ""

START_TIME=$(date +%s)

# Run converter with partitioned output and index generation
"$CONVERTER_BIN" \
    --input "$CHANGESET_FILE" \
    --output-dir "$DATA_DIR" \
    --partition \
    --generate-indexes \
    --continue-on-error

CONVERT_EXIT=$?
END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

if [ $CONVERT_EXIT -ne 0 ]; then
    echo -e "${RED}Error: Conversion failed with exit code $CONVERT_EXIT${NC}"
    exit $CONVERT_EXIT
fi

echo -e "${GREEN}✓ Conversion complete (took ${DURATION}s)${NC}"

# Clean up download if requested
if [ -z "$INPUT_FILE" ] && [ "$KEEP_DOWNLOAD" = false ]; then
    echo -e "\n${YELLOW}Cleaning up download...${NC}"
    rm -f "$CHANGESET_FILE"
    echo -e "${GREEN}✓ Removed temporary file${NC}"
fi

# Show results
echo -e "\n${GREEN}=== Initialization Complete ===${NC}"
echo ""
echo "Output directory: $DATA_DIR"
echo "Contents:"
find "$DATA_DIR" -type f -name "*.parquet" | head -10 | while read -r file; do
    SIZE=$(du -h "$file" | cut -f1)
    echo "  - $(basename "$file") ($SIZE)"
done

TOTAL_FILES=$(find "$DATA_DIR" -type f -name "*.parquet" | wc -l)
TOTAL_SIZE=$(du -sh "$DATA_DIR" 2>/dev/null | cut -f1 || echo "unknown")
echo ""
echo "Total: $TOTAL_FILES parquet files, $TOTAL_SIZE"

if [ -f "$DATA_DIR/metadata.json" ]; then
    echo ""
    echo "Metadata available at: $DATA_DIR/metadata.json"
fi

echo ""
echo "Next steps:"

# Check if we need to fix ownership
if [ -d "$DATA_DIR" ]; then
    CURRENT_OWNER=$(stat -c '%U' "$DATA_DIR" 2>/dev/null || stat -f '%Su' "$DATA_DIR" 2>/dev/null || echo "unknown")
    if [ "$CURRENT_OWNER" != "chch" ] && id chch &>/dev/null; then
        echo "  ${YELLOW}1. Fix ownership: sudo chown -R chch:chch $DATA_DIR${NC}"
        echo "  2. Start/restart services: sudo systemctl start chch-server chch-worker"
    else
        echo "  1. Start/restart services: sudo systemctl start chch-server chch-worker"
    fi
else
    echo "  1. Start/restart services: sudo systemctl start chch-server chch-worker"
fi

echo "  3. Check service status: sudo systemctl status chch-server chch-worker"
echo "  4. Test API: curl http://localhost:3000/health"
echo "  5. Query changesets: curl 'http://localhost:3000/changesets?bbox=-180,-90,180,90&limit=10'"
