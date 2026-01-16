# OSM Changeset API Load Testing Framework

Minimal load testing framework to establish performance baselines and measure optimization impact.

## Overview

This framework tests three critical performance aspects:

1. **Connection Contention** - Tests the `Arc<Mutex<Connection>>` bottleneck
2. **Memory Stress** - Tests large result set buffering (7-day queries)
3. **Mixed Traffic** - Simulates realistic production load

## Prerequisites

- **k6** - Load testing tool ([install instructions](https://k6.io/docs/getting-started/installation/))
  ```bash
  # macOS
  brew install k6

  # Linux (Debian/Ubuntu)
  sudo gpg -k
  sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A3642D57D77C6C491D6AC1D69
  echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
  sudo apt-get update
  sudo apt-get install k6
  ```

- **Node.js** - For result analysis (optional but recommended)
  ```bash
  # macOS
  brew install node

  # Linux
  sudo apt-get install nodejs npm
  ```

- **Running API server** - The changeset API must be running
  ```bash
  cd /path/to/changesets-to-parquet
  cargo run --release --bin changeset-server
  ```

## Quick Start

```bash
# Make sure API server is running on localhost:3000
cd loadtest

# Run baseline test (takes ~47 minutes)
./run.sh baseline

# Results will be saved in:
# - results/json/baseline_TIMESTAMP.json
# - results/csv/baseline_TIMESTAMP.csv
# - results/baseline_TIMESTAMP-summary.txt
```

## Test Scenarios

### Scenario A: Connection Contention (17 minutes)

Tests the mutex-locked DuckDB connection bottleneck.

- **Pattern**: Single changeset lookups (`/changeset/{id}.json`)
- **Load**: Ramps from 10 → 100 concurrent virtual users
- **Purpose**: Find where mutex serialization causes degradation
- **Expected**: Degradation around 40-60 concurrent requests

**Stages:**
```
10 VUs  (2 min) → Baseline
20 VUs  (2 min) → Light load
40 VUs  (2 min) → Expected bottleneck
60 VUs  (2 min) → High load
100 VUs (2 min) → Stress
```

### Scenario B: Memory Stress (14 minutes)

Tests memory buffering with large 7-day date range queries.

- **Pattern**: 7-day date ranges on recent dates (`/changesets/date/from/.../to/...`)
- **Load**: Ramps from 2 → 10 concurrent queries
- **Purpose**: Find memory limits with result set buffering
- **Expected**: OOM risk at 5-10 concurrent 7-day queries

**Stages:**
```
2 VUs  (3 min) → Gentle start
5 VUs  (3 min) → Medium stress
10 VUs (3 min) → High stress (likely OOM)
```

### Scenario C: Mixed Realistic Traffic (16 minutes)

Simulates production-like request distribution.

- **Traffic Mix**:
  - 20% Health checks
  - 10% `/meta/latest` (cached)
  - 30% Single changeset lookups
  - 25% Day queries
  - 10% Hour queries
  - 5% Small date ranges (1-3 days)

- **Load**: Ramps from 10 → 100 requests/second
- **Purpose**: Realistic production simulation
- **Expected**: System saturates at 40-60 req/sec

**Stages:**
```
10 req/sec  (2 min) → Baseline
20 req/sec  (2 min) → Light
40 req/sec  (2 min) → Expected limit
60 req/sec  (2 min) → High
100 req/sec (2 min) → Stress
```

## Usage

### Basic Usage

```bash
# Run with default settings (localhost:3000)
./run.sh baseline

# Run against different URL
BASE_URL=http://example.com:3000 ./run.sh baseline

# Run with custom name
./run.sh my-test-run
```

### Manual k6 Execution

```bash
# Run k6 directly with JSON output
k6 run \
  --out json=results/json/manual.json \
  -e BASE_URL=http://localhost:3000 \
  scenarios/baseline-test.js

# Analyze results
node analysis/analyze.js results/json/manual.json manual
```

### Single Scenario Testing

Edit `scenarios/baseline-test.js` to comment out scenarios you don't want to run:

```javascript
export const options = {
  scenarios: {
    // Uncomment only the scenario you want
    connection_contention: { ... },
    // memory_stress: { ... },
    // mixed_traffic: { ... }
  }
};
```

## Output Files

### JSON Output (`results/json/*.json`)

Raw k6 metrics in newline-delimited JSON format. Each line contains a metric data point.

**Fields:**
- `type`: Metric type (Point, Metric, etc.)
- `metric`: Metric name (http_req_duration, http_reqs, etc.)
- `data.value`: Metric value
- `data.tags`: Tags (endpoint, scenario, status, etc.)
- `data.time`: Timestamp

### CSV Output (`results/csv/*.csv`)

Processed metrics in CSV format for spreadsheet analysis.

**Columns:**
- `timestamp`: ISO 8601 timestamp
- `metric`: Metric name
- `value`: Metric value
- `endpoint`: Endpoint name/type
- `scenario`: Scenario name
- `status`: HTTP status code
- `error`: Error message (if any)

**Import into Excel/Google Sheets:**
1. Open your spreadsheet application
2. Import CSV file
3. Create pivot tables:
   - Rows: Scenario or Endpoint
   - Values: Average/Percentiles of http_req_duration
4. Create charts for visualization

### Summary Text (`results/*-summary.txt`)

Human-readable summary with key statistics.

**Sections:**
- Overall statistics (total requests, success rate, percentiles)
- Statistics by scenario
- Statistics by endpoint type
- Error summary

## Analyzing Results

### Key Metrics to Look For

**Connection Contention Test:**
- **p95 response time vs VUs**: Should be flat at low VUs, then increase sharply
- **Inflection point**: Where response time starts degrading (expected: 40-60 VUs)
- **Throughput saturation**: Max req/sec before degradation

**Memory Stress Test:**
- **Response size**: Typical 7-day query response size
- **p95 response time**: Should increase with concurrent queries
- **Error rate**: OOM errors at high concurrency
- **Memory limit**: Number of concurrent queries before failure

**Mixed Traffic Test:**
- **Overall throughput**: Sustainable req/sec
- **p95 by endpoint type**: Which endpoints are slowest
- **Error rate at load**: When system becomes unreliable

### Spreadsheet Analysis

1. **Create response time chart**:
   - X-axis: Time or VU count
   - Y-axis: http_req_duration
   - Lines: p50, p90, p95, p99

2. **Create throughput chart**:
   - X-axis: Time
   - Y-axis: Requests per second
   - Compare target vs actual

3. **Create endpoint comparison**:
   - Bar chart of p95 by endpoint type
   - Identify slowest endpoints

### Comparing Before/After

```bash
# Run baseline before changes
./run.sh before-pooling

# Make changes (e.g., add connection pooling)
# ...

# Run comparison test
./run.sh after-pooling

# Compare summaries
diff results/before-pooling_*-summary.txt results/after-pooling_*-summary.txt
```

## Understanding Bottlenecks

### Arc<Mutex<Connection>> Bottleneck

**Symptom**: Response time increases linearly with concurrent requests

**Evidence:**
- p95 response time flat up to ~20 VUs, then increases
- Throughput plateaus around 200-300 req/sec
- Connection contention scenario shows clear degradation

**Fix**: Connection pooling (see main README)

### Memory Buffering Bottleneck

**Symptom**: OOM errors or very slow responses on large queries

**Evidence:**
- 7-day queries take >5 seconds
- Error rate increases with concurrent large queries
- System becomes unresponsive at 5-10 concurrent large queries

**Fix**: Streaming, pagination, or result size limits

### DuckDB Query Performance

**Symptom**: Even single queries are slow

**Evidence:**
- Single changeset lookups take >500ms
- Day queries on old dates (low volume) still slow
- CPU usage is high during queries

**Fix**: Query optimization, indexes, or caching

## Troubleshooting

### "k6 is not installed"

Install k6 following the instructions above.

### "API server is not responding"

Make sure the API server is running:
```bash
cargo run --release --bin changeset-server
```

Check the health endpoint manually:
```bash
curl http://localhost:3000/health
```

### "Analysis failed"

The analysis requires Node.js. Install it or skip analysis:
```bash
# macOS
brew install node

# Or just use the JSON output directly
```

### Test takes too long

Run individual scenarios by editing `scenarios/baseline-test.js`:
- Comment out scenarios you don't need
- Reduce stage durations
- Reduce target VUs/req rates

### Out of memory errors

This is expected in the memory stress test! This indicates the memory limit.

If it happens in other scenarios, reduce concurrent load.

## Next Steps

After running baseline tests:

1. **Review summary** - Identify primary bottlenecks
2. **Compare with predictions** - Were bottlenecks where expected?
3. **Implement fixes** - Start with connection pooling
4. **Run comparison tests** - Measure improvement
5. **Document findings** - Update PERFORMANCE.md with results

## File Structure

```
loadtest/
├── lib/
│   ├── config.js       # Configuration constants
│   ├── utils.js        # Helper functions (date generation, etc.)
│   └── metrics.js      # Custom k6 metrics
├── scenarios/
│   └── baseline-test.js # Main test scenarios
├── analysis/
│   └── analyze.js      # Results processor
├── results/            # Output directory (gitignored except baseline)
│   ├── json/           # Raw k6 JSON output
│   └── csv/            # Processed CSV files
├── run.sh              # Test runner script
└── README.md           # This file
```

## Contributing

When adding new test scenarios:

1. Create scenario in `scenarios/baseline-test.js`
2. Add appropriate thresholds
3. Update this README with scenario description
4. Run and validate before committing

## References

- [k6 Documentation](https://k6.io/docs/)
- [k6 Scenarios](https://k6.io/docs/using-k6/scenarios/)
- [k6 Metrics](https://k6.io/docs/using-k6/metrics/)
- [../docs/PERFORMANCE.md](../docs/PERFORMANCE.md) - Performance analysis results
- [../MEMORY_ANALYSIS.md](../MEMORY_ANALYSIS.md) - Memory usage analysis
