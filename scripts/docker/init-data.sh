#!/bin/bash
set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
DATA_DIR="${DATA_DIR:-${HOME}/data/chch}"
DOWNLOAD_DIR="${DOWNLOAD_DIR:-${HOME}/downloads/chch}"
PLANET_URL="${PLANET_URL:-https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2}"
CONVERTER_IMAGE="${CONVERTER_IMAGE:-chch-converter:latest}"

# Parse arguments
SKIP_DOWNLOAD=false
INPUT_FILE=""
KEEP_DOWNLOAD=false

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Initialize ch-ch-changes data using Docker converter.

Options:
    -i, --input FILE        Use existing changeset file instead of downloading
    -s, --skip-download     Skip download if file already exists
    -k, --keep              Keep downloaded file after conversion
    -d, --data-dir DIR      Output directory (default: ~/data/chch)
    -u, --url URL           Planet dump URL (default: OSM changesets-latest)
    -h, --help              Show this help message

Examples:
    # Download and convert latest planet dump
    $0

    # Use existing file
    $0 --input ~/changesets.osm.bz2

    # Custom data directory
    $0 --data-dir /mnt/data/chch

Environment Variables:
    DATA_DIR         Output directory (default: ~/data/chch)
    DOWNLOAD_DIR     Download directory (default: ~/downloads/chch)
    PLANET_URL       Download URL (default: OSM changesets-latest)
    CONVERTER_IMAGE  Docker image name (default: chch-converter:latest)
EOF
    exit 1
}

# Parse arguments
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

echo -e "${BLUE}=== ch-ch-changes Docker Initial Data Load ===${NC}"
echo ""

# Check if Docker is available
if ! command -v docker &> /dev/null; then
    echo -e "${RED}Error: Docker not found${NC}"
    echo "Please install Docker first"
    exit 1
fi

# Check if converter image exists, build if needed
if ! docker image inspect "$CONVERTER_IMAGE" &> /dev/null; then
    echo -e "${YELLOW}Converter image not found. Building...${NC}"
    REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    docker build -f "$REPO_DIR/Dockerfile.converter" -t "$CONVERTER_IMAGE" "$REPO_DIR"
    echo -e "${GREEN}✓ Built converter image${NC}"
fi

echo "Configuration:"
echo "  Converter image: $CONVERTER_IMAGE"
echo "  Data dir:        $DATA_DIR"
echo "  Download dir:    $DOWNLOAD_DIR"
echo ""

# Create directories
mkdir -p "$DATA_DIR"
mkdir -p "$DOWNLOAD_DIR"

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
    CHANGESET_FILE="$DOWNLOAD_DIR/$(basename "$PLANET_URL")"

    if [ -f "$CHANGESET_FILE" ] && [ "$SKIP_DOWNLOAD" = true ]; then
        echo -e "${YELLOW}Found existing file, skipping download: $CHANGESET_FILE${NC}"
        FILE_SIZE=$(du -h "$CHANGESET_FILE" | cut -f1)
        echo "  Size: $FILE_SIZE"
    else
        echo -e "${YELLOW}[1/2] Downloading planet changeset dump...${NC}"
        echo "  URL: $PLANET_URL"
        echo "  Destination: $CHANGESET_FILE"
        echo ""

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

# Convert the data
echo -e "\n${YELLOW}[2/2] Converting changeset data to Parquet...${NC}"
echo "This may take 30-60 minutes for full planet history..."
echo ""

START_TIME=$(date +%s)

# Get absolute paths for Docker volume mounts
INPUT_DIR=$(dirname "$(cd "$(dirname "$CHANGESET_FILE")" && pwd)/$(basename "$CHANGESET_FILE")")
INPUT_FILENAME=$(basename "$CHANGESET_FILE")
OUTPUT_DIR=$(cd "$DATA_DIR" && pwd)

docker run --rm \
    -v "$INPUT_DIR:/input:ro" \
    -v "$OUTPUT_DIR:/data" \
    "$CONVERTER_IMAGE" \
    --input "/input/$INPUT_FILENAME" \
    --output-dir /data \
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
find "$DATA_DIR" -type f -name "*.parquet" 2>/dev/null | head -10 | while read -r file; do
    SIZE=$(du -h "$file" | cut -f1)
    echo "  - $(basename "$file") ($SIZE)"
done

TOTAL_FILES=$(find "$DATA_DIR" -type f -name "*.parquet" 2>/dev/null | wc -l)
TOTAL_SIZE=$(du -sh "$DATA_DIR" 2>/dev/null | cut -f1 || echo "unknown")
echo ""
echo "Total: $TOTAL_FILES parquet files, $TOTAL_SIZE"

if [ -f "$DATA_DIR/metadata.json" ]; then
    echo ""
    echo "Metadata: $DATA_DIR/metadata.json"
fi

echo ""
echo "Next steps:"
echo "  1. Update .env file with DATA_PATH=$DATA_DIR"
echo "  2. Start services: docker-compose up -d"
echo "  3. Check logs: docker-compose logs -f"
echo "  4. Test API: curl http://localhost:3000/health"
