# Local Development Guide

Quick guide to running the OSM changeset API locally for development and testing.

## Prerequisites

- Rust (2024)
- ~10GB disk space
- DuckDB CLI for direct queries (optional)

## Quick Start (5 minutes)

### 1. Clone and Build

```bash
git clone https://codeberg.org/mvexel/ch-ch-changes.git
cd ch-ch-changes

# Build all binaries
cargo build --release
```

This creates three binaries in `target/release/`:
- `ch-ch-changes` - Convert OSM data to Parquet
- `chch-server` - API server
- `chch-worker` - Real-time updates

### 2. Get Sample Data

Download a recent changeset dump (or use a small sample):

```bash
# Create data directory
mkdir -p data/changesets/base

# Option A: Download latest full dump (~2GB compressed, ~6GB extracted)
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Option B: Use a smaller sample for testing
# Download a single day's replication file instead
wget https://planet.openstreetmap.org/replication/changesets/004/953/state.yaml
wget https://planet.openstreetmap.org/replication/changesets/004/953/001.osm.gz
```

### 3. Convert to Parquet

```bash
# For full dump (takes 5-15 minutes)
./target/release/ch-ch-changes \
    --input changesets-latest.osm.bz2 \
    --output-dir data/changesets/base \
    --partition \
    --generate-indexes

# For sample file
bunzip2 -c 001.osm.gz | ./target/release/ch-ch-changes \
    --input - \
    --output-dir data/changesets/base \
    --partition
```

This creates partitioned Parquet files:
```
data/changesets/base/
  year=2024/
    month=01/
      *.parquet
    month=02/
      *.parquet
  ...
```

### 4. Start the API Server

```bash
DATA_PATH=data/changesets ./target/release/chch-server
```

Server starts on `http://localhost:3000`

### 5. Test It

```bash
# Health check
curl http://localhost:3000/health

# Get changesets from a specific hour
curl "http://localhost:3000/changesets?time=2025-01-15T12"

# Get changesets in a time range
curl "http://localhost:3000/changesets?start=2025-01-15T10:00:00Z&end=2025-01-15T12:00:00Z"

# Get by changeset ID range
curl "http://localhost:3000/changesets?start_id=123456&end_id=123500"

# JSON Lines format (for streaming)
curl "http://localhost:3000/changesets?time=2025-01-15T12&format=jsonl"

# XML format
curl "http://localhost:3000/changesets?time=2025-01-15T12&format=xml"
```

## Development Workflow

### Running with Auto-Reload

Use `cargo-watch` for auto-reload during development:

```bash
# Install cargo-watch
cargo install cargo-watch

# Run server with auto-reload
DATA_PATH=data/changesets cargo watch -x 'run --bin chch-server'
```

### Testing Changes

```bash
# Run all tests
cargo test

# Run specific package tests
cargo test -p chch-server
cargo test -p ch-ch-changes

# Run with output
cargo test -- --nocapture
```

### Check Code Quality

```bash
# Format code
cargo fmt

# Lint
cargo clippy -- -D warnings

# Check without building
cargo check --all-targets
```

## Adding Real-Time Updates (Optional)

To get live minutely updates from OSM:

### 1. Bootstrap State

Find the latest sequence number in your base data:

```bash
# Query the max changeset ID and closed_at from your Parquet files
duckdb -c "SELECT MAX(id) as max_id, MAX(closed_at) as max_time 
           FROM 'data/changesets/base/**/*.parquet'"
```

Then look up the corresponding replication sequence at:
https://planet.openstreetmap.org/replication/changesets/

Or bootstrap automatically:

```bash
DATA_PATH=data/changesets \
./target/release/chch-worker \
    --bootstrap-state-from-parquet
```

### 2. Start Replication Worker

```bash
# Run once to test
DATA_PATH=data/changesets \
./target/release/chch-worker \
    --once

# Run continuously (checks every 60 seconds by default)
DATA_PATH=data/changesets \
POLL_INTERVAL_SECONDS=60 \
./target/release/chch-worker
```

This creates delta files:
```
data/changesets/delta/
  year=2025/
    month=01/
      seq-004953001.parquet
      seq-004953060.parquet
  state.yaml
```

The API server automatically picks up new delta files!

## Configuration

Environment variables for `chch-server`:

```bash
# Data directory (required)
DATA_PATH=data/changesets

# Server port (default: 3000)
PORT=3000

# Bind address (default: 0.0.0.0)
HOST=127.0.0.1

# Log level (default: info)
RUST_LOG=debug
```

Environment variables for `chch-worker`:

```bash
# Data directory (required)
DATA_PATH=data/changesets

# Poll interval in seconds (default: 60)
POLL_INTERVAL_SECONDS=60

# Bootstrap from this sequence (first run only)
REPLICATION_START_SEQUENCE=4953001

# Or bootstrap from timestamp
REPLICATION_START_TIMESTAMP=2025-01-15T00:00:00Z
```

## Project Structure

```
ch-ch-changes/
├── crates/
│   ├── chch-server/        # API server
│   ├── ch-ch-changes/   # Converter binary
│   ├── chch-worker/  # Replication daemon
│   └── ch-ch-changes-core/         # Shared library
├── scripts/
│   ├── deploy.sh               # Production deployment
│   ├── rollback.sh             # Version rollback
│   └── healthcheck.sh          # Health monitoring
├── systemd/                    # systemd service files
└── data/                       # Local data directory
    └── changesets/
        ├── base/               # Full planet dump (Parquet)
        └── delta/              # Minutely updates (Parquet)
```

## Querying with DuckDB

You can query the Parquet files directly:

```bash
# Install DuckDB
brew install duckdb  # macOS
# or download from https://duckdb.org/

# Query your local files
duckdb -c "SELECT COUNT(*) FROM 'data/changesets/base/**/*.parquet'"

# Find your changesets
duckdb -c "SELECT id, created_at, num_changes 
           FROM 'data/changesets/base/**/*.parquet' 
           WHERE user = 'YOUR_USERNAME' 
           ORDER BY created_at DESC 
           LIMIT 10"

# Changesets by editor
duckdb -c "SELECT 
             COALESCE(created_by, 'Unknown') as editor,
             COUNT(*) as count
           FROM 'data/changesets/base/**/*.parquet'
           GROUP BY editor
           ORDER BY count DESC
           LIMIT 10"

# Changesets per month
duckdb -c "SELECT 
             strftime(created_at, '%Y-%m') as month,
             COUNT(*) as changesets,
             SUM(num_changes) as total_changes
           FROM 'data/changesets/base/**/*.parquet'
           GROUP BY month
           ORDER BY month"
```

## Troubleshooting

### "No parquet files found"

Make sure:
- `DATA_PATH` points to the parent of `base/` and `delta/` directories
- Parquet files exist in `$DATA_PATH/base/year=YYYY/month=MM/*.parquet`
- Files have `.parquet` extension

```bash
# Check structure
tree -L 4 data/changesets/
```

### Server returns empty results

- Check time ranges in your data:
  ```bash
  duckdb -c "SELECT MIN(created_at), MAX(created_at) 
             FROM 'data/changesets/base/**/*.parquet'"
  ```
- Query times must match your data range
- Time queries are limited to 1 day maximum

### Port already in use

Change the port:
```bash
PORT=3001 DATA_PATH=data/changesets ./target/release/chch-server
```

### Build errors

Update Rust:
```bash
rustup update stable
```

Clear build cache:
```bash
cargo clean
cargo build --release
```

## API Examples

See the full API at `http://localhost:3000/docs` (Swagger UI)

### Time-based queries

```bash
# Specific hour
curl "http://localhost:3000/changesets?time=2025-01-15T12"

# Specific minute
curl "http://localhost:3000/changesets?time=2025-01-15T12:30"

# Time range (max 1 day)
curl "http://localhost:3000/changesets?start=2025-01-15T00:00:00Z&end=2025-01-15T23:59:59Z"
```

### ID-based queries

```bash
# Single ID
curl "http://localhost:3000/changesets/123456"

# ID range
curl "http://localhost:3000/changesets?start_id=123456&end_id=123500"
```

### Output formats

```bash
# JSON (default)
curl "http://localhost:3000/changesets?time=2025-01-15T12"

# JSON Lines (streaming)
curl "http://localhost:3000/changesets?time=2025-01-15T12&format=jsonl"

# XML (OSM format)
curl "http://localhost:3000/changesets?time=2025-01-15T12&format=xml"
```

## Performance Tips

- **Use JSONL for large results** - streams data instead of buffering
- **Partition by time** - queries are fastest on specific hours/days
- **Generate indexes** - use `--generate-indexes` when converting
- **SSD recommended** - significantly faster than HDD for Parquet queries

## Next Steps

- **Production deployment**: See `scripts/README.md`
- **Replication setup**: See `_local/REPLICATION_WORKER.md`
- **Query examples**: See `_local/QUERY_EXAMPLES.md`
- **Architecture**: See `_local/ARCHITECTURE.md`

## Getting Help

- Open an issue: https://codeberg.org/mvexel/ch-ch-changes/issues
- OSM Wiki: https://wiki.openstreetmap.org/wiki/Changesets
- DuckDB docs: https://duckdb.org/docs/
