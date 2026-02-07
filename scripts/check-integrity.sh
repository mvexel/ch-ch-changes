#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;36m'
NC='\033[0m' # No Color

DATA_PATH="${1:-/opt/chch/data}"

echo -e "${BLUE}=== Changeset Data Integrity Check ===${NC}"
echo "Checking data at: $DATA_PATH"
echo ""

# Check if DuckDB CLI is available
if ! command -v duckdb &> /dev/null; then
    echo -e "${RED}Error: duckdb CLI not found.${NC}"
    echo ""
    echo "Install it with:"
    echo "  # Ubuntu/Debian"
    echo "  wget https://github.com/duckdb/duckdb/releases/latest/download/duckdb_cli-linux-amd64.zip"
    echo "  unzip duckdb_cli-linux-amd64.zip"
    echo "  sudo mv duckdb /usr/local/bin/"
    echo ""
    echo "  # macOS"
    echo "  brew install duckdb"
    exit 1
fi

# Create temporary SQL script
TEMP_SQL=$(mktemp)
trap "rm -f $TEMP_SQL" EXIT

cat > "$TEMP_SQL" <<'SQL'
-- Query to find gaps in changeset IDs
WITH changeset_ranges AS (
    SELECT 
        MIN(id) as min_id,
        MAX(id) as max_id,
        COUNT(*) as count,
        MAX(id) - MIN(id) + 1 as expected_count
    FROM read_parquet([
        '$DATA_PATH/base/year=*/month=*/*.parquet',
        '$DATA_PATH/delta/**/*.parquet'
    ])
),
gaps AS (
    SELECT 
        id as gap_start,
        LEAD(id) OVER (ORDER BY id) as next_id,
        LEAD(id) OVER (ORDER BY id) - id - 1 as gap_size
    FROM read_parquet([
        '$DATA_PATH/base/year=*/month=*/*.parquet',
        '$DATA_PATH/delta/**/*.parquet'
    ])
    ORDER BY id
)
SELECT 
    'Summary' as check_type,
    min_id::VARCHAR as value1,
    max_id::VARCHAR as value2,
    count::VARCHAR as value3,
    (expected_count - count)::VARCHAR as value4
FROM changeset_ranges
UNION ALL
SELECT 
    'Gap' as check_type,
    gap_start::VARCHAR as value1,
    next_id::VARCHAR as value2,
    gap_size::VARCHAR as value3,
    '' as value4
FROM gaps
WHERE gap_size > 0
ORDER BY check_type DESC, value1::BIGINT
LIMIT 100;
SQL

# Replace placeholder with actual path (handle both macOS and Linux sed)
if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s|\$DATA_PATH|$DATA_PATH|g" "$TEMP_SQL"
else
    sed -i "s|\$DATA_PATH|$DATA_PATH|g" "$TEMP_SQL"
fi

echo -e "${YELLOW}Running integrity checks (this may take a minute)...${NC}"
echo ""

# Debug: show the query
if [[ "${DEBUG:-}" == "1" ]]; then
    echo "Generated SQL:"
    cat "$TEMP_SQL"
    echo ""
fi

# Run the query
set +e  # Temporarily disable exit on error to capture output
RESULT=$(duckdb -csv -noheader < "$TEMP_SQL" 2>&1)
EXIT_CODE=$?
set -e

if [[ $EXIT_CODE -ne 0 ]]; then
    echo -e "${RED}Error running integrity check (exit code: $EXIT_CODE):${NC}"
    echo "$RESULT"
    echo ""
    echo "Debug info:"
    echo "  SQL file: $TEMP_SQL"
    echo "  Data path: $DATA_PATH"
    echo "  Base dir exists: $([ -d "$DATA_PATH/base" ] && echo "yes" || echo "no")"
    echo "  Delta dir exists: $([ -d "$DATA_PATH/delta" ] && echo "yes" || echo "no")"
    echo ""
    echo "Try running with DEBUG=1 to see the generated SQL"
    exit 1
fi

if [[ -z "$RESULT" ]]; then
    echo -e "${YELLOW}Warning: No results returned from query${NC}"
    echo "This might mean:"
    echo "  - No parquet files found in $DATA_PATH/base or $DATA_PATH/delta"
    echo "  - Files don't match the expected patterns"
    echo ""
    echo "Found parquet files:"
    find "$DATA_PATH/base" -name "*.parquet" 2>/dev/null | head -5
    find "$DATA_PATH/delta" -name "*.parquet" 2>/dev/null | head -5
    exit 1
fi

# Parse and display results
echo "$RESULT" | while IFS=',' read -r check_type value1 value2 value3 value4; do
    if [[ "$check_type" == "Summary" ]]; then
        echo -e "${BLUE}Data Summary:${NC}"
        echo "  Earliest ID:     $value1"
        echo "  Latest ID:       $value2"
        echo "  Total Count:     $value3"
        echo "  Missing IDs:     $value4"
        echo ""
        if [[ "$value4" -gt 0 ]]; then
            echo -e "${YELLOW}⚠ Found $value4 missing changeset IDs${NC}"
        else
            echo -e "${GREEN}✓ No gaps detected (continuous sequence)${NC}"
        fi
        echo ""
    elif [[ "$check_type" == "Gap" ]]; then
        if [[ -z "${GAPS_HEADER:-}" ]]; then
            echo -e "${RED}Gaps Detected:${NC}"
            echo "  Start ID → End ID (Gap Size)"
            GAPS_HEADER=1
        fi
        echo -e "  ${RED}$value1 → $value2 ($value3 missing)${NC}"
    fi
done

# Check replication state
echo ""
echo -e "${BLUE}Replication State:${NC}"
if [[ -f "$DATA_PATH/state.yaml" ]]; then
    echo "$(cat $DATA_PATH/state.yaml)"
else
    echo -e "${YELLOW}No state file found${NC}"
fi

# Check latest OSM replication sequence
echo ""
echo -e "${BLUE}Latest OSM Sequence:${NC}"
LATEST_OSM=$(curl -s https://planet.openstreetmap.org/replication/changesets/state.yaml | grep sequenceNumber | awk '{print $2}')
echo "  OSM Server: $LATEST_OSM"

if [[ -f "$DATA_PATH/state.yaml" ]]; then
    LOCAL_SEQ=$(grep sequenceNumber "$DATA_PATH/state.yaml" | awk '{print $2}')
    echo "  Local Data: $LOCAL_SEQ"
    BEHIND=$((LATEST_OSM - LOCAL_SEQ))
    if [[ $BEHIND -gt 100 ]]; then
        echo -e "  ${RED}Behind by: $BEHIND sequences${NC}"
    elif [[ $BEHIND -gt 10 ]]; then
        echo -e "  ${YELLOW}Behind by: $BEHIND sequences${NC}"
    else
        echo -e "  ${GREEN}Behind by: $BEHIND sequences (OK)${NC}"
    fi
fi

echo ""
echo -e "${GREEN}=== Integrity Check Complete ===${NC}"
