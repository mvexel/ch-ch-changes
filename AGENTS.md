# AI Agent Guide: Changesets to Parquet

## Mission
This project provides high-performance access to OpenStreetMap (OSM) changeset metadata. It consists of:
1.  **Converter**: Efficiently transforms raw OSM XML dumps (`.osm.bz2`) into querying-optimized Parquet files.
2.  **Replication**: A worker daemon that fetches live minutely updates from OSM and appends them as hourly delta Parquet files.
3.  **API**: A read-only HTTP server (Axum) allowing users to query this data via REST endpoints or SQL-like logic, backed by DuckDB.

## 🗺️ Architectural Map

### Cargo Workspace Structure

The project is organized as a Cargo workspace with four crates:

```
changesets-to-parquet/
├── Cargo.toml                          # Workspace root
├── crates/
│   ├── changesets-core/                # Shared library
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # Changeset struct + module exports
│   │       ├── schema.rs               # Arrow/Parquet schema (source of truth)
│   │       ├── parsing.rs              # XML → Rust streaming parser
│   │       ├── writer.rs               # Partitioned Parquet file writing
│   │       ├── builder.rs              # Arrow RecordBatch builders
│   │       ├── models.rs               # Partition keys, stats
│   │       └── indexing.rs             # Optional index generation (feature-gated)
│   │
│   ├── changesets-to-parquet/          # Converter CLI
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   │
│   ├── changeset-replication-worker/   # Replication daemon
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   │
│   └── changeset-server/               # HTTP API
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── auth/                   # OAuth + API key authentication
│           ├── query/                  # DuckDB query engine
│           ├── routes/                 # REST endpoints
│           ├── format/                 # JSON/JSONL/XML serialization
│           ├── models/                 # API-specific data models
│           ├── state.rs
│           └── constants.rs
```

### Crate Responsibilities

| Crate | Purpose | Key Dependencies |
|-------|---------|------------------|
| **changesets-core** | Shared types and logic | arrow, parquet, quick-xml, serde, duckdb (optional) |
| **changesets-to-parquet** | CLI converter for bulk planet dumps | changesets-core, bzip2, clap |
| **changeset-replication-worker** | Daemon for minutely replication | changesets-core, reqwest, flate2, duckdb |
| **changeset-server** | HTTP API server | changesets-core, axum, tokio, duckdb, r2d2 |

**Important:** The `indexing` module in `changesets-core` is behind the `indexing` feature flag and is only enabled for the converter binary.

## 🛠️ Development & Workflows

### Build & Run

```bash
# Build all workspace crates
cargo build --workspace --release

# Run specific binary
cargo run -p changesets-to-parquet -- --input test.osm.bz2 --output-dir ./output --partition
cargo run -p changeset-replication-worker -- --help
cargo run -p changeset-server

# Run tests
cargo test --workspace

# Build individual crate
cd crates/changesets-core
cargo build --features indexing
```

### Load Testing
Located in `loadtest/`. Uses `k6`.
```bash
# Install k6
brew install k6

# Run baseline test
k6 run loadtest/scenarios/baseline-test.js
```

### Database & Auth
The API uses a local SQLite database for authentication (`auth.db` by default).
-   **Middleware**: `crates/changeset-server/src/auth/middleware.rs` handles `Authorization: Bearer <key>` validation.

## 📐 Coding Conventions

-   **Error Handling**: Use `anyhow` for app-level errors. Specific library errors should be wrapped or mapped.
-   **Async Runtime**: `tokio` is the runtime. The API is fully async; the converter and worker are synchronous/threaded CPU-bound work.
-   **Logging**: Use the `tracing` crate. `info!` for general ops, `debug!` for verbose flow, `error!` for failures.
-   **Dependencies**: Managed at workspace level in root `Cargo.toml`. Individual crates use `workspace = true` for version consistency.

## 🔑 Key Files to Know

| File Path | Purpose |
| :--- | :--- |
| `crates/changesets-core/src/schema.rs` | **Source of Truth** for the Parquet schema. Edit this to add columns. |
| `crates/changesets-core/src/parsing.rs` | Logic for turning XML tags into struct fields. |
| `crates/changesets-core/src/lib.rs` | Canonical Changeset struct definition. |
| `crates/changeset-server/src/routes/changesets.rs` | Main logic for querying/filtering changesets. |
| `crates/changeset-server/src/query/engine.rs` | Where DuckDB SQL queries are constructed and executed. |
| `crates/changeset-replication-worker/src/main.rs` | Logic for handling state and fetching upstream updates. |

## 🧠 Context for AI
-   **Data Model**: "Changesets" are metadata about a set of edits in OSM. They contain a user, timestamp, bbox, and tags (key-value pairs).
-   **Performance**: The system is designed for high throughput. The API relies on DuckDB's ability to query Parquet files directly without loading everything into memory.
-   **Partitioning**: Data is partitioned by time (Year/Month). Delta files are hourly (YYYYMMDDHH.parquet).
-   **Deduplication**: Query-time dedup uses `replication_sequence` (higher = newer; base is NULL).
-   **Indexes**: No partition stats index; filesystem paths are the only index.

## 🔧 Implementation Details

### Replication Worker Internals
-   **Network Resilience**: The `retry_network` function (lines 19-69 in `crates/changeset-replication-worker/src/main.rs`) handles transient failures with exponential backoff. Retries on timeouts, connection errors, request/body errors, and 5xx/429 responses. Max 5 retries with 2s initial backoff.
-   **HTTP Client Config**: Uses 180s timeout, 30s connect timeout, TCP keepalive (60s), and connection pooling to maintain stable connections to planet.openstreetmap.org.
-   **BatchAccumulator**: Aggregates changesets across multiple replication sequences into hourly batches. Flushes on hour boundary crossings or when `max_batch_sequences` (default 120) is reached.
-   **State Management**: OSM's replication state files are off-by-one (state file for sequence N contains sequence N-1). The worker normalizes this internally.

### API Query Engine
-   **QueryEngine** (`crates/changeset-server/src/query/engine.rs`): Wraps DuckDB connection pool. Uses `r2d2` for connection management.
-   **Partition Discovery** (`crates/changeset-server/src/query/partition.rs`): Watches filesystem for new Parquet files using `notify`. Caches partition metadata.
-   **Format Support**: JSON (default), JSONL (streaming), XML. See `crates/changeset-server/src/format/` for serialization logic.

### Authentication Flow
1.  User visits `/auth/osm/start` → redirected to OSM OAuth
2.  OSM redirects back to `/auth/osm/callback` with auth code
3.  Server exchanges code for access token, fetches user info
4.  API key generated and stored in SQLite (`AUTH_DB_PATH`)
5.  Subsequent requests use `Authorization: Bearer <key>` header

## 🐛 Common Issues

| Symptom | Likely Cause | Fix |
| :--- | :--- | :--- |
| "connection closed before message completed" | Network instability to OSM servers | Already handled by retry logic; increase `MAX_RETRIES` if persistent |
| Replication falls behind | Too many sequences queued | Use `--max-sequences-per-run` to limit catch-up batches |
| "state missing sequenceNumber" | Corrupt or invalid state file | Delete `state.yaml` and set `REPLICATION_START_SEQUENCE` |
| Query returns stale data | Partition cache not refreshed | Server watches for file changes; check filesystem permissions |
| OAuth callback fails | Misconfigured `OSM_OAUTH_REDIRECT_URL` | Ensure URL matches exactly what's registered at osm.org |
| Build error: "use of unresolved module `indexing`" | Building without indexing feature | Add `--features indexing` for converter builds |

## 📁 Data Directory Structure

```
$DATA_PATH/
├── base/                           # From converter (full dump)
│   └── year=YYYY/
│       └── month=MM/
│           └── data.parquet
├── delta/                          # From replication worker
│   ├── year=YYYY/
│   │   └── month=MM/
│   │       └── YYYYMMDDHH.parquet  # Hourly batches
│   ├── state.yaml                  # Current replication position
│   └── tmp/                        # Working directory for downloads
└── indexes/                        # Optional DuckDB indexes (if enabled)
```

## 🔄 Data Flow

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  OSM Planet     │────▶│    Converter     │────▶│  base/*.parquet │
│  Dump (.bz2)    │     │  (CLI Binary)    │     │                 │
└─────────────────┘     └──────────────────┘     └────────┬────────┘
                                                          │
┌─────────────────┐     ┌──────────────────┐     ┌────────▼────────┐
│  OSM Replication│────▶│ Replication      │────▶│ delta/*.parquet │
│  Feed (minutely)│     │ Worker (Daemon)  │     │                 │
└─────────────────┘     └──────────────────┘     └────────┬────────┘
                                                          │
                        ┌──────────────────┐     ┌────────▼────────┐
                        │   API Server     │◀────│   DuckDB        │
                        │   (Axum/Tokio)   │     │   (in-memory)   │
                        └────────┬─────────┘     └─────────────────┘
                                 │
                        ┌────────▼─────────┐
                        │   HTTP Clients   │
                        │   (JSON/XML/JSONL)
                        └──────────────────┘
```

## 🚀 Deployment

### Building Release Binaries

```bash
# Build all binaries for deployment
cargo build --release --workspace

# Binaries are located at:
# - target/release/changesets-to-parquet
# - target/release/changeset-replication-worker
# - target/release/changeset-server
```

### Systemd Services

Service files are in `systemd/`:
- `changeset-server.service` - API server
- `changeset-replication-worker.service` - Replication daemon

### Deployment Scripts

Located in `scripts/`:
- `setup.sh` - Initial server setup (creates user, directories, systemd service)
- `deploy.sh` - Build and deploy new version with zero-downtime swap
- `rollback.sh` - Rollback to previous version
- `process_changesets.sh` - One-time full planet dump processing
- `healthcheck.sh` - Health check script for monitoring

**Deployment workflow:**
```bash
# Initial setup (run once)
sudo ./scripts/setup.sh

# Deploy new version
./scripts/deploy.sh

# Rollback if needed
./scripts/rollback.sh <version>
```

The deployment script:
1. Builds all workspace binaries in release mode
2. Creates versioned release directory
3. Stops service
4. Updates symlinks atomically
5. Restarts service
6. Runs health checks

## 📦 Binary Paths After Deployment

After running `scripts/deploy.sh`, binaries are installed at:
```
/opt/changeset-server/bin/changeset-server
/opt/changeset-server/bin/changesets-to-parquet
/opt/changeset-server/bin/changeset-replication-worker
```

And symlinked to:
```
/usr/local/bin/changeset-server
/usr/local/bin/changesets-to-parquet
/usr/local/bin/changeset-replication-worker
```

## 🔧 Environment Variables

See `systemd/changeset-server.service` for required environment variables:
- `DATA_PATH` - Path to Parquet data directory
- `AUTH_DB_PATH` - SQLite database path for API keys
- `AUTH_KEY_SECRET` - Secret for API key hashing
- `OSM_OAUTH_CLIENT_ID` - OAuth client ID
- `OSM_OAUTH_CLIENT_SECRET` - OAuth client secret
- `OSM_OAUTH_REDIRECT_URL` - OAuth callback URL
- `REQUIRE_AUTH` - Enable/disable authentication (default: true)
