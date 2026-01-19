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
