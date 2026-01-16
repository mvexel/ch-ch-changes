# Performance Optimization Guide

This document describes the performance optimizations implemented for the OSM Changeset API and how to configure your system for optimal performance.

## Overview

The API was optimized to address critical bottlenecks identified through systematic load testing. Key improvements include connection pooling, query safety limits, and system configuration tuning.

## Baseline Performance (Before Optimizations)

Load testing with `Arc<Mutex<Connection>>` (single mutex-locked connection) revealed severe bottlenecks:

### Metrics (23,550 requests over 47 minutes)
- **Success Rate**: 91.67% (1,961 failures)
- **Mean Response Time**: 18.7 seconds
- **Median Response Time**: 12.8 seconds
- **p95 Response Time**: 60 seconds (timeout)
- **p99 Response Time**: 60 seconds (timeout)
- **Throughput**: ~8-10 requests/second

### Issues Identified

1. **Mutex Serialization Bottleneck**
   - All queries serialized through single `Arc<Mutex<Connection>>`
   - Even health checks took 20+ seconds waiting for mutex
   - 100 concurrent requests = 99 waiting, 1 executing
   - No parallelism despite 4 CPU cores

2. **File Descriptor Exhaustion**
   - DuckDB opens multiple parquet files per query
   - Hit macOS default limit of 256 file descriptors
   - Caused "Too many open files" errors
   - Queries failed or hung waiting for file handles

3. **Memory Buffering Issues**
   - All results buffered in memory before sending
   - 7-day queries on recent dates = 500MB+ per request
   - 10 concurrent large queries = 5GB → OOM on 4GB system

## Optimizations Implemented

### 1. Connection Pooling (CRITICAL)

**Problem**: Single `Arc<Mutex<Connection>>` serialized all queries.

**Solution**: Replaced with r2d2 connection pool (default: 4 connections).

**Implementation**:
```rust
// Before
pub struct QueryEngine {
    conn: Arc<Mutex<Connection>>,  // ❌ Bottleneck
}

// After
pub struct QueryEngine {
    pool: Pool<DuckDBConnectionManager>,  // ✅ Parallel queries
}
```

**Expected Impact**:
- Response times: **18s → <500ms** (36x improvement)
- Throughput: **10 req/sec → 200+ req/sec** (20x improvement)
- Success rate: **92% → 99%+**
- Parallel query execution across CPU cores

**Files Changed**:
- `Cargo.toml`: Added `r2d2` dependency
- `api/query/engine.rs`: Implemented `DuckDBConnectionManager` and connection pool

### 2. Query Safety Limits

**Problem**: Large queries caused OOM and system instability.

**Solution**: Added protective limits with clear error messages.

**Limits Implemented**:

#### 7-Day Maximum Date Range
```rust
const MAX_DAYS: i64 = 7;
if (end_date - start_date).num_days() > MAX_DAYS {
    return ApiError::bad_request_with_details(
        "Date range too large",
        "Maximum allowed is 7 days. Use smaller ranges or pagination."
    );
}
```

#### 100k Maximum Result Set Size
```rust
const MAX_RESULT_SIZE: usize = 100_000;
if changesets.len() > MAX_RESULT_SIZE {
    return ApiError::bad_request_with_details(
        "Result set too large",
        "Use smaller date range, filters, or pagination."
    );
}
```

**Files Changed**:
- `api/routes/changesets.rs`: Added safety checks to date range handler

### 3. File Descriptor Limit Increase

**Problem**: System default (256 on macOS) insufficient for concurrent parquet file access.

**Solution**: Increase file descriptor limit before starting server.

**Required**: See [System Configuration](#system-configuration) below.

## System Configuration

### Increase File Descriptor Limit

**Required for proper operation.** DuckDB opens multiple parquet files concurrently.

#### macOS/Linux (Per Session)

```bash
# Check current limit
ulimit -n

# Increase to 4096 (temporary, current shell only)
ulimit -n 4096

# Then start server
DATA_PATH=/path/to/data cargo run --release --bin changeset-server
```

#### macOS (Permanent)

Edit `/etc/sysctl.conf` (requires sudo):
```bash
kern.maxfiles=65536
kern.maxfilesperproc=32768
```

Edit `/etc/launchd.conf`:
```bash
limit maxfiles 65536 200000
```

Reboot for changes to take effect.

#### Linux (Permanent)

Edit `/etc/security/limits.conf`:
```
*  soft  nofile  65536
*  hard  nofile  65536
```

Or for systemd services, add to service file:
```ini
[Service]
LimitNOFILE=65536
```

#### Verify After Restart

```bash
ulimit -n
# Should show 4096 or higher
```

### Connection Pool Size Tuning

**Default**: 4 connections

**Tuning Guidelines**:
- Start with 4 (1 per CPU core)
- Increase for higher concurrency needs
- Monitor memory usage (each connection has overhead)
- Typical range: 4-16 connections

**Custom Pool Size**:
```rust
// In api/main.rs when creating QueryEngine
let engine = QueryEngine::new_with_pool_size(data_path, 8)?;
```

## Testing Performance

### Run Baseline Test

```bash
cd loadtest
./run.sh baseline
```

**Output**:
- `results/baseline_TIMESTAMP.json` - Raw k6 metrics
- `results/csv/baseline_TIMESTAMP.csv` - For charting
- `results/baseline_TIMESTAMP-summary.txt` - Key statistics

### Run Comparison Test (After Optimizations)

```bash
cd loadtest
./run.sh after-pooling
```

### Compare Results

```bash
# View summaries side by side
diff results/baseline_*-summary.txt results/after-pooling_*-summary.txt
```

**Metrics to Compare**:
- Mean/median/p95/p99 response times
- Requests per second (throughput)
- Success rate (should be 99%+)
- Error counts by endpoint

## Expected Production Performance

### With All Optimizations

**Single Changeset Lookups**:
- Response time: <100ms p95
- Throughput: 200-400 req/sec
- Success rate: 99.9%+

**Day Queries** (single day):
- Response time: <1s p95 (recent dates with high volume)
- Response time: <200ms p95 (older dates with low volume)
- Throughput: 20-50 req/sec

**Date Range Queries** (up to 7 days):
- Response time: <3s p95
- Throughput: 5-10 req/sec
- Limited by result set size, not concurrency

**Health Checks**:
- Response time: <10ms p95
- Throughput: 1000+ req/sec

### Bottlenecks Remaining

1. **DuckDB Query Execution**
   - Even with pooling, DuckDB queries are computationally expensive
   - Consider caching for frequently accessed date ranges
   - See [Future Optimizations](#future-optimizations)

2. **Parquet File I/O**
   - Disk I/O can bottleneck for queries spanning many partitions
   - SSD recommended over HDD
   - Consider SSD caching for hot partitions

3. **User Queries**
   - Still require full table scans (no user-based partitioning)
   - Will always be slower than date/ID-based queries
   - Consider separate user index if these queries are common

## Monitoring

### Key Metrics to Track

**Response Times**:
```bash
# Watch server logs for slow queries
tail -f server.log | grep "Query error\|took"
```

**File Descriptor Usage**:
```bash
# Check current open files (macOS)
lsof -p $(pgrep changeset-server) | wc -l

# Linux
ls /proc/$(pgrep changeset-server)/fd | wc -l
```

**Memory Usage**:
```bash
# Monitor memory
ps aux | grep changeset-server

# Or use top/htop
top -pid $(pgrep changeset-server)
```

**Connection Pool**:
- Monitor connection pool saturation in logs
- If frequently at max connections, consider increasing pool size

### Warning Signs

**File Descriptor Issues**:
- Error: "Too many open files"
- Action: Increase `ulimit -n` and restart

**Memory Pressure**:
- Queries timing out
- Slow response times
- Action: Check for queries exceeding safety limits, reduce concurrent load

**Connection Pool Saturation**:
- High p95/p99 latency
- Requests queueing
- Action: Increase pool size or optimize slow queries

## Future Optimizations

### 1. Query Result Caching

**Use Case**: Frequently accessed date ranges or high-traffic endpoints.

**Implementation**: Add moka cache layer:
```rust
use moka::future::Cache;

pub struct QueryEngine {
    pool: Pool<DuckDBConnectionManager>,
    cache: Cache<QueryKey, Arc<Vec<Changeset>>>,
}
```

**Expected Impact**: 10-100x improvement for cache hits.

### 2. Re-enable Streaming

**Problem**: Currently disabled due to OOM issues.

**Solution**: Implement proper backpressure with connection pooling:
- Stream results as they're fetched from DuckDB
- Don't buffer entire result set in memory
- Use tokio channels with bounded capacity

**Expected Impact**: Support unlimited result sizes without OOM.

### 3. Hot/Cold Data Separation (CQRS)

**Use Case**: If recent data (last 7 days) accounts for >80% of queries.

**Implementation**:
- Recent data in Redis/PostgreSQL (fast queries)
- Historical data in DuckDB/Parquet (analytics)
- Route queries based on date range

**Expected Impact**: 10-50x improvement for recent data queries.

### 4. Read Replicas

**Use Case**: Very high query load (1000+ req/sec).

**Implementation**:
- Multiple API server instances
- Load balancer distributing traffic
- Shared read-only access to parquet files

**Expected Impact**: Linear scaling with instance count.

## Troubleshooting

### Performance Regression

**Symptom**: Response times increase over time.

**Check**:
1. File descriptor limit: `ulimit -n` (should be 4096+)
2. Connection pool size: Review logs for saturation
3. Disk space: Ensure adequate free space for parquet files
4. Memory: Check for memory leaks or excessive usage

### High Error Rate

**Symptom**: >5% request failure rate.

**Check**:
1. Server logs: Look for specific error messages
2. File descriptors: `lsof -p $(pgrep changeset-server) | wc -l`
3. Memory: `ps aux | grep changeset-server`
4. DuckDB errors: Check for query failures in logs

### Slow Queries

**Symptom**: Specific endpoints consistently slow.

**Investigate**:
1. Query complexity: Check SQL generated by QueryBuilder
2. Partition count: Queries spanning many partitions are slower
3. Result set size: Large responses take longer to serialize
4. Concurrency: Check if connection pool is saturated

## Load Testing

See `loadtest/README.md` for complete load testing documentation.

**Quick Start**:
```bash
cd loadtest
./run.sh baseline          # Before optimizations
./run.sh after-pooling     # After optimizations
```

**Compare Results**:
- Check summary files for p50/p95/p99 percentiles
- Import CSV to spreadsheet for charting
- Focus on throughput and error rates

## References

- Baseline test results: `loadtest/results/baseline_*-summary.txt`
- Load testing framework: `loadtest/README.md`
- Memory analysis: `MEMORY_ANALYSIS.md`
- Rust performance patterns: Plan file in `.claude/plans/`

---

**Last Updated**: 2026-01-16
**Optimizations Applied**: Connection pooling, query safety limits, file descriptor tuning
