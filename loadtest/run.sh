#!/bin/bash
set -euo pipefail

# Load Test Runner Script
# Executes k6 baseline tests and generates analysis reports

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
BASE_URL="${BASE_URL:-http://localhost:3000}"
RUN_NAME="${1:-baseline}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_PREFIX="${RUN_NAME}_${TIMESTAMP}"

# Paths
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
RESULTS_DIR="${SCRIPT_DIR}/results"
JSON_DIR="${RESULTS_DIR}/json"
CSV_DIR="${RESULTS_DIR}/csv"

# Ensure directories exist
mkdir -p "${JSON_DIR}"
mkdir -p "${CSV_DIR}"

# Output files
JSON_OUTPUT="${JSON_DIR}/${OUTPUT_PREFIX}.json"
CSV_OUTPUT="${CSV_DIR}/${OUTPUT_PREFIX}.csv"
SUMMARY_OUTPUT="${RESULTS_DIR}/${OUTPUT_PREFIX}-summary.txt"

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}K6 Load Test Runner${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "Run Name:    ${RUN_NAME}"
echo "Timestamp:   ${TIMESTAMP}"
echo "Base URL:    ${BASE_URL}"
echo "Output:      ${OUTPUT_PREFIX}"
echo ""

# Check if k6 is installed
if ! command -v k6 &> /dev/null; then
    echo -e "${RED}ERROR: k6 is not installed${NC}"
    echo "Install k6: https://k6.io/docs/getting-started/installation/"
    echo ""
    echo "On macOS: brew install k6"
    echo "On Linux: sudo apt-get install k6"
    exit 1
fi

# Check if server is running
echo -e "${YELLOW}Checking if API server is running...${NC}"
if ! curl -s -f "${BASE_URL}/health" > /dev/null 2>&1; then
    echo -e "${RED}ERROR: API server is not responding at ${BASE_URL}${NC}"
    echo "Please start the API server before running load tests."
    echo ""
    echo "To start the server:"
    echo "  cargo run --release --bin changeset-server"
    exit 1
fi
echo -e "${GREEN}✓ API server is running${NC}"
echo ""

# Run k6 test
echo -e "${YELLOW}Starting k6 load test...${NC}"
echo "This will take approximately 47 minutes (if all 3 scenarios enabled)"
echo ""
echo "Scenarios:"
echo "  1. Connection Contention (0-17 min)"
echo "  2. Memory Stress (17-31 min)"
echo "  3. Mixed Traffic (31-47 min)"
echo ""

k6 run \
  --out "json=${JSON_OUTPUT}" \
  -e BASE_URL="${BASE_URL}" \
  "${SCRIPT_DIR}/scenarios/baseline-test.js"

K6_EXIT_CODE=$?

if [ $K6_EXIT_CODE -ne 0 ]; then
    echo -e "${RED}k6 test failed with exit code ${K6_EXIT_CODE}${NC}"
    echo "Check the output above for errors."
    echo ""
    echo "JSON output saved to: ${JSON_OUTPUT}"
    exit $K6_EXIT_CODE
fi

echo -e "${GREEN}✓ k6 test completed successfully${NC}"
echo ""

# Check if Node.js is installed for analysis
if ! command -v node &> /dev/null; then
    echo -e "${YELLOW}WARNING: Node.js is not installed${NC}"
    echo "Skipping analysis. Install Node.js to generate CSV and summary."
    echo ""
    echo "JSON output saved to: ${JSON_OUTPUT}"
    exit 0
fi

# Run analysis
echo -e "${YELLOW}Analyzing results...${NC}"
node "${SCRIPT_DIR}/analysis/analyze.js" "${JSON_OUTPUT}" "${OUTPUT_PREFIX}"

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Analysis complete${NC}"
    echo ""
    echo "Results:"
    echo "  JSON:    ${JSON_OUTPUT}"
    echo "  CSV:     ${CSV_OUTPUT}"
    echo "  Summary: ${SUMMARY_OUTPUT}"
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}Test Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
else
    echo -e "${RED}Analysis failed${NC}"
    echo "JSON output is available at: ${JSON_OUTPUT}"
    exit 1
fi
