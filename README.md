# ch-ch-changes

![bowie](https://chch.osm.lol/static/CH-CH-CHANGES.png)

*The missing API for OSM Changeset Metadata queries.*

The OpenStreetMap API [lets you query changeset metadata](https://wiki.openstreetmap.org/wiki/API_v0.6#Query:_GET_/api/0.6/changesets), but is limited to 100 results per query.

This API lets you query by entire days, hours, minutes, or ID ranges, and supports multiple output formats (XML, JSON, JSONL) for efficient batch processing.

## Quick Start

### With Docker (recommended)

```bash
# 1. Load initial data (required first!)
./scripts/docker/init-data.sh

# 2. Start server and worker
docker-compose up -d

# 3. Check logs
docker-compose logs -f

# 4. Access API
curl http://localhost:3000/health
```

See [DEPLOYMENT.md](DEPLOYMENT.md) for the complete deployment guide.

### Without Docker

Requires Python 3.11+

```bash
# Install
pip install -e .

# Load initial data (required first!)
./scripts/init-data.sh

# Start server
chch-server

# Or run worker
chch-worker
```

See [DEPLOYMENT.md](DEPLOYMENT.md) for systemd deployment guide (advanced).

## API Examples

```bash
# Health check
curl http://localhost:3000/health

# All changesets on a specific day (JSON)
curl http://localhost:3000/changesets/day/20240115.json

# Changesets in a specific hour (XML)
curl http://localhost:3000/changesets/day/20240115/hour/14

# Changesets by ID range with filters
curl 'http://localhost:3000/changesets/id/from/100000/to/200000.json?user=alice&limit=100'

# Dataset statistics
curl http://localhost:3000/meta/state
```

## About Changesets

Each time an OpenStreetMap contributor uploads edits, a **changeset** is created.
A changeset has two components:

- **Metadata** — when it was opened/closed, by whom, editor used, and contextual tags (this API)
- **Map changes** — the actual edits to OSM data

This API provides the metadata only.

## Architecture

The system consists of three components:

### 1. Converter (`chch-convert`)
Bulk imports OSM planet changeset dumps into Parquet files.

**Use the Rust converter for planet dumps** (40GB+ XML files):

```bash
# Requires: Rust toolchain (cargo)
cd rust-converter
cargo run --release --bin chch-convert -- \
  --input changesets-latest.osm.bz2 \
  --output-dir /data/base \
  --partition
```

The Rust converter is 5-10x faster and optimized for large files. See [rust-converter/README.md](rust-converter/README.md) for details. A Python converter (`chch-convert-py`) is also available for small files or testing.

### 2. Server (`chch-server`)
FastAPI (Python) server that queries Parquet files using DuckDB.

- Database-less: queries Parquet files directly
- Serves API endpoints and landing page
- Auto-discovers new data files

### 3. Worker (`chch-worker`)
Python worker that continuously fetches minutely replication data from OSM.

- Polls OSM replication feed every 60s
- Batches minutely data into hourly Parquet files
- Automatically compacts old files
- Smart backfill from base layer cutoff timestamp

## Technical Details

### Database-less Architecture

- Hive-partitioned Parquet files (`year=YYYY/month=MM/*.parquet`)
- Queried directly using embedded DuckDB
- No indexes, no traditional DB
- Works on local disk, NFS, or S3-compatible storage

### Lambda Architecture

- **Base layer** — immutable weekly planet snapshots
- **Delta layer** — minutely replication batched into hourly Parquet

Queries read both layers and deduplicate via `replication_sequence`.

### Smart Batching

Replication data is flushed when:

- Hour boundary is crossed
- 120 sequences accumulate
- Worker shuts down

This avoids millions of tiny files while keeping data fresh.

### Automatic Discovery

The server scans for new Parquet files every 30 seconds and automatically serves them.

### Trade-offs and Quirks

- Date ranges limited to 1 day
- ID ranges limited to 1M span, 100k results
- Deduplication happens at query-time
- No spatial queries or SQL interface
- Read-only fixed endpoints

## Data Flow

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  OSM Planet     │────▶│    Converter     │────▶│  base/*.parquet │
│  Dump (.bz2)    │     │                  │     │                 │
└─────────────────┘     └──────────────────┘     └────────┬────────┘
                                                          │
┌─────────────────┐     ┌──────────────────┐     ┌────────▼────────┐
│  OSM Replication│────▶│ Replication      │────▶│ delta/*.parquet │
│  Feed (minutely)│     │ Worker           │     │                 │
└─────────────────┘     └──────────────────┘     └────────┬────────┘
                                                          │
                        ┌──────────────────┐     ┌────────▼────────┐
                        │   API Server     │◀────│   DuckDB        │
                        │   (FastAPI)      │     │   (in-process)  │
                        └────────┬─────────┘     └─────────────────┘
                                 │
                        ┌────────▼─────────┐
                        │   HTTP Clients   │
                        │   (JSON/XML/JSONL│
                        └──────────────────┘
```

## Deployment

See detailed deployment guides:

- **[Docker deployment](scripts/docker/README.md)** (recommended)
- **[systemd deployment](scripts/systemd/README.md)** (without Docker)

### Quick Start: Full Setup

**Step 1: Initial data load (one-time, use Rust converter)**

```bash
# Download latest planet changeset dump
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Convert to Parquet (takes 2-4 hours for full history)
cd rust-converter
cargo run --release --bin chch-convert -- \
  --input ../changesets-latest.osm.bz2 \
  --output-dir /data/base \
  --partition
cd ..
```

**Step 2: Start server and worker (Python)**

```bash
# Using Docker (recommended)
docker-compose up -d

# Or using systemd
sudo ./scripts/systemd/setup.sh
sudo systemctl start chch-server chch-worker
```

The worker will automatically detect your base layer cutoff date and backfill from there to present.

### Production Setup

1. Clone repository
2. Load initial data using Rust converter (see above)
3. Configure environment variables in `.env`
4. Deploy with Docker Compose or systemd
5. Put behind reverse proxy (Caddy, Nginx) for HTTPS
6. Monitor `/health` and `/meta/state` endpoints

Example Caddyfile:

```
chch.osm.lol {
    reverse_proxy localhost:3000
}
```

## Development

```bash
# Install with dev dependencies
pip install -e ".[dev]"

# Run tests
pytest tests/ -v

# Lint and format
ruff check src/ tests/
ruff format src/ tests/

# Start server with hot reload
uvicorn ch_ch_changes.server.app:create_app --factory --reload
```

## Configuration

All configuration via environment variables:

```bash
# Server
DATA_PATH=/path/to/data          # Where parquet files live
HOST=0.0.0.0                     # Listen address
PORT=3000                        # HTTP port
LOG_LEVEL=INFO                   # DEBUG, INFO, WARNING, ERROR

# Worker
POLL_INTERVAL_SECONDS=60         # How often to check for new data
MAX_SEQUENCES_PER_RUN=1000       # Max sequences per poll
HOURLY_BATCHING=true             # Batch minutely data into hourly files
ENABLE_COMPACTION=true           # Merge old hourly files
COMPACTION_AGE_HOURS=24          # Only compact files older than this
```

## Data Directory Structure

```
/data/
├── base/                          # Base layer (planet dumps)
│   └── year=YYYY/month=MM/*.parquet
├── delta/                         # Delta layer (replication)
│   ├── year=YYYY/month=MM/*.parquet
│   └── state.yaml                 # Replication state
└── metadata.json                  # Dataset statistics (auto-generated)
```

## See Also

- [OpenStreetMap planet downloads](https://planet.osm.org) — source data
- [Changeset wiki page](https://wiki.openstreetmap.org/wiki/Changeset) — about changesets
- [OSMCha](https://osmcha.org) — inspect actual map edits

## License

ISC License

Data from [OpenStreetMap](https://www.openstreetmap.org) © OpenStreetMap contributors,
available under the [Open Database License](https://opendatacommons.org/licenses/odbl/).
