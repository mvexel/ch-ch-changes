# AI Agent Guide: Changesets to Parquet

## Mission
This project provides high-performance access to OpenStreetMap (OSM) changeset metadata. It consists of:
1.  **Converter**: Efficiently transforms raw OSM XML dumps (`.osm.bz2`) into querying-optimized Parquet files.
2.  **Replication**: A worker daemon that fetches live minutely updates from OSM and appends them as hourly delta Parquet files.
3.  **API**: A read-only HTTP server (Axum) allowing users to query this data via REST endpoints or SQL-like logic, backed by DuckDB.

## 🗺️ architectural Map

### 1. Core Logic (`src/`)
Shared library code used by both the converter and the worker.
-   `parsing.rs`: Streaming XML parser (using `quick-xml`) that converts OSM XML events into Rust structs.
-   `schema.rs`: Defines the Arrow/Parquet schema for changesets.
-   `writer.rs`: Handles writing Arrow batches to Parquet files, including partitioning logic.

### 2. Binaries
-   **Converter** (`src/main.rs`): CLI tool for bulk processing.
-   **Worker** (`src/bin/changeset-replication-worker.rs`): Daemon for keeping data up-to-date.
-   **API Server** (`api/main.rs`): The public-facing web server.

### 3. API Structure (`api/`)
-   `auth/`: Authentication logic using SQLite to manage API keys.
-   `query/`: DuckDB query engine wrappers.
-   `routes/`: API endpoint definitions (REST).
-   `models/`: API-specific data models (serde structs).

### 4. Infrastructure (`scripts/`, `systemd/`)
-   Deployment scripts (`deploy.sh`, `rollback.sh`).
-   Systemd unit files for running the server and worker as services.

## 🛠️ Development & Workflows

### Build & Run
```bash
# Build all binaries
cargo build --release

# Run Unit Tests
cargo test

# Run API Server (Local)
DATA_PATH=./data cargo run --bin changeset-server

# Run Converter (Example)
cargo run --bin changesets-to-parquet -- --input test.osm.bz2 --output output.parquet
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
-   **Middleware**: `api/auth/middleware.rs` handles `Authorization: Bearer <key>` validation.

## 📐 Coding Conventions

-   **Error Handling**: Use `anyhow` for app-level errors. Specific library errors should be wrapped or mapped.
-   **Async Runtime**: `tokio` is the runtime. The API is fully async; the converter is largely synchronous/threaded CPU-bound work.
-   **Logging**: Use the `tracing` crate. `info!` for general ops, `debug!` for verbose flow, `error!` for failures.
-   **Dependencies**: Check `Cargo.toml`. `duckdb` is used for querying Parquet files. `axum` is the web framework.

## 🔑 Key Files to Know

| File Path | Purpose |
| :--- | :--- |
| `src/schema.rs` | **Source of Truth** for the Parquet schema. Edit this to add columns. |
| `src/parsing.rs` | Logic for turning XML tags into struct fields. |
| `api/routes/changesets.rs` | Main logic for querying/filtering changesets. |
| `api/query/engine.rs` | Where DuckDB SQL queries are constructed and executed. |
| `src/bin/changeset-replication-worker.rs` | Logic for handling state and fetching upstream updates. |

## 🧠 Context for AI
-   **Data Model**: "Changesets" are metadata about a set of edits in OSM. They contain a user, timestamp, bbox, and tags (key-value pairs).
-   **Performance**: The system is designed for high throughput. The API relies on DuckDB's ability to query Parquet files directly without loading everything into memory.
-   **Partitioning**: Data is partitioned by time (Year/Month). Delta files are hourly (YYYYMMDDHH.parquet).
-   **Deduplication**: Query-time dedup uses `replication_sequence` (higher = newer; base is NULL).
-   **Indexes**: No partition stats index; filesystem paths are the only index.

## 🔧 Implementation Details

### Replication Worker Internals
-   **Network Resilience**: The `retry_network` function (lines 19-69 in `changeset-replication-worker.rs`) handles transient failures with exponential backoff. Retries on timeouts, connection errors, request/body errors, and 5xx/429 responses. Max 5 retries with 2s initial backoff.
-   **HTTP Client Config**: Uses 180s timeout, 30s connect timeout, TCP keepalive (60s), and connection pooling to maintain stable connections to planet.openstreetmap.org.
-   **BatchAccumulator**: Aggregates changesets across multiple replication sequences into hourly batches. Flushes on hour boundary crossings or when `max_batch_sequences` (default 120) is reached.
-   **State Management**: OSM's replication state files are off-by-one (state file for sequence N contains sequence N-1). The worker normalizes this internally.

### API Query Engine
-   **QueryEngine** (`api/query/engine.rs`): Wraps DuckDB connection pool. Uses `r2d2` for connection management.
-   **Partition Discovery** (`api/query/partition.rs`): Watches filesystem for new Parquet files using `notify`. Caches partition metadata.
-   **Format Support**: JSON (default), JSONL (streaming), XML. See `api/format/` for serialization logic.

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

## 📁 Data Directory Structure

```
$DATA_PATH/
├── base/                           # From converter (full dump)
│   └── year=YYYY/
│       └── month=MM.parquet
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
                        │   (Axum)         │     │   (in-memory)   │
                        └────────┬─────────┘     └─────────────────┘
                                 │
                        ┌────────▼─────────┐
                        │   HTTP Clients   │
                        │   (JSON/XML/JSONL│
                        └──────────────────┘
```
