# DuckDB Query Engine for Parquet Files

This document describes the `QueryEngine` - a DuckDB-based wrapper for querying OSM changeset Parquet files.

`HISTORIC_API_PLAN.md` is the reference document for the API; this file reflects the current `api/query/` implementation.

## Overview

The query engine provides a high-level interface to efficiently query changeset data stored in Parquet files organized by year/month partitions. It handles:

- Connection management to DuckDB
- Partition-aware querying for optimal performance
- Type-safe conversion of Parquet rows to Rust structs
- Common query patterns (by ID, date range, user, etc.)
- Filtering and pagination

Note: The QueryEngine lives under `api/query/` and is used by the `changeset-server` binary. It is not exported from the library crate; examples below assume usage inside the API server crate (so module paths start with `crate::`).

## Architecture

### Components

The implementation is split into two modules:

#### `api/query/engine.rs` - QueryEngine

The main query interface providing methods for:
- Single changeset lookup by ID
- Date range queries
- ID range queries
- User-based queries
- Open/closed changeset queries
- Custom SQL queries
- Counting changesets

#### `api/query/partition.rs` - Partition Management

Helper functions for working with the partition directory structure:
- Listing available partitions
- Finding partitions for date/ID ranges
- Building SQL glob patterns
- Parsing partition metadata

## Usage

### Basic Setup

```rust
use crate::query::QueryEngine;

// Create a query engine pointing to your parquet data directory
let engine = QueryEngine::new("/path/to/parquet/data")?;

// The data directory should have this structure:
// data/
//   year=2023/
//     month=01.parquet
//     month=02.parquet
//   year=2024/
//     month=01.parquet
//     ...
```

### Query by ID

```rust
// Get a single changeset by its ID
if let Some(changeset) = engine.get_changeset(168575829)? {
    println!("ID: {}", changeset.id);
    println!("User: {:?}", changeset.user);
}
```

### Date Range Queries

```rust
use chrono::{DateTime, Utc, TimeZone};
use crate::query::QueryFilters;

// Query all changesets created in May 2024
let start = Utc.with_ymd_and_hms(2024, 5, 1, 0, 0, 0).unwrap();
let end = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();

let changesets = engine.get_changesets_by_date_range(
    &start.to_rfc3339(),
    &end.to_rfc3339(),
    &QueryFilters::new(),
)?;
println!("Found {} changesets", changesets.len());
```

### Date Range with Filters

```rust
use crate::query::QueryFilters;

let filters = QueryFilters::new()
    .with_username("mvexel".to_string())
    .with_open(false) // Only closed changesets
    .with_limit(100)
    .with_offset(0);

let changesets = engine.get_changesets_by_date_range(
    &start.to_rfc3339(),
    &end.to_rfc3339(),
    &filters,
)?;
```

### Query by User

```rust
use crate::query::QueryFilters;

let filters = QueryFilters::new();
let changesets = engine.get_changesets_by_user("mvexel", &filters)?;
```

### Open/Closed Changesets (via filters)

```rust
use crate::query::QueryFilters;

let filters = QueryFilters::new().with_open(true);
let open = engine.get_changesets_by_date_range(
    &start.to_rfc3339(),
    &end.to_rfc3339(),
    &filters,
)?;
```

### ID Range Queries

```rust
use crate::query::QueryFilters;

// Get all changesets with IDs between two values
let changesets = engine.get_changesets_by_id_range(100000, 100100, &QueryFilters::new())?;

// With filters
let filters = QueryFilters::new().with_username("mvexel".to_string());
let changesets = engine.get_changesets_by_id_range(100000, 100100, &filters)?;
```

### Counting

```rust
// Count total changesets
let total = engine.get_total_count()?;
println!("Total changesets: {}", total);

// Get latest changeset ID
let latest = engine.get_latest_id()?;
println!("Latest changeset ID: {}", latest);
```

## Data Types

### Changeset

```rust
pub struct Changeset {
    pub id: i64,
    pub created_at: Option<i64>,      // milliseconds since epoch
    pub closed_at: Option<i64>,       // milliseconds since epoch
    pub open: bool,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub min_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lat: Option<f64>,
    pub max_lon: Option<f64>,
    pub num_changes: u32,
    pub comments_count: u32,
    pub description: Option<String>,
    pub tags: HashMap<String, String>,
}
```

### QueryFilters

```rust
pub struct QueryFilters {
    pub username: Option<String>,  // Filter by username
    pub uid: Option<i64>,          // Filter by user ID
    pub open: Option<bool>,        // None = no filter, true = open only, false = closed only
    pub limit: Option<u32>,        // Maximum number of results
    pub offset: Option<u32>,       // Number of results to skip
}

// All fields are optional - use QueryFilters::new() for no filters
let no_filters = QueryFilters::new();
```

## Partition Management

`PartitionResolver` exists in `api/query/partition.rs` but is not implemented yet. It will provide partition listing and pruning once completed.

## Performance Considerations

### Partitioning Strategy

The changeset data is organized by year/month to enable efficient queries:

```
data/
├── year=2023/
│   ├── month=01.parquet
│   ├── month=02.parquet
│   └── ...
├── year=2024/
│   ├── month=01.parquet
│   └── ...
```

When querying by date range, the engine automatically identifies which partition files to query, reducing I/O overhead.

### Query Optimization Tips

1. **Use Date Ranges When Possible**: Date-range queries can leverage partition elimination
   ```rust
   use crate::query::QueryFilters;

   // Good - uses partitions from May only
   engine.get_changesets_by_date_range(
       &may_start.to_rfc3339(),
       &may_end.to_rfc3339(),
       &QueryFilters::new(),
   )?;
   ```

2. **Filter Before Pagination**: Apply filters before using limit/offset
   ```rust
   let filters = QueryFilters::new()
       .with_username("mvexel".to_string())
       .with_limit(100);
   ```

## Error Handling

The `QueryEngine` uses the `anyhow::Result` type for error handling:

```rust
use crate::query::QueryEngine;
use anyhow::{Context, Result};

fn query_example() -> Result<()> {
    let engine = QueryEngine::new("/nonexistent/path")
        .context("Failed to create query engine")?;

    let filters = crate::query::QueryFilters::new();
    let start = "2024-01-01T00:00:00Z";
    let end = "2024-01-02T00:00:00Z";
    let changesets = engine
        .get_changesets_by_date_range(start, end, &filters)
        .context("Failed to query changesets")?;

    Ok(())
}
```

## Integration with the API Server

The `QueryEngine` is designed to be used with the API server to provide HTTP endpoints:

```rust
// In an axum route handler
use crate::query::QueryEngine;
use axum::{extract::Path, Json};

async fn get_changeset_handler(
    Path(id): Path<i64>,
) -> Result<Json<Changeset>> {
    let engine = QueryEngine::new("./data")?;

    match engine.get_changeset(id)? {
        Some(changeset) => Ok(Json(changeset)),
        None => Err(/* not found */),
    }
}
```

## Testing

Query engine tests are minimal right now. When added, run:

```bash
# Run all query engine tests
cargo test api::query

```

## Dependencies

The query engine uses:
- `duckdb` (v1.1+) - SQL engine for Parquet files
- `chrono` - Date/time handling
- `anyhow` - Error handling

The `duckdb` crate is compiled with the `bundled` feature, so no external DuckDB installation is needed.

## Limitations and Future Work

### Current Limitations

1. **In-Memory DuckDB Connection**: Currently creates an in-memory DuckDB instance per engine
   - Could be optimized with connection pooling for repeated queries

2. **Limited Optimization**: ID-range queries scan all partitions
   - Date-range queries also scan all partitions until `PartitionResolver` is implemented
   - Could be improved with partition pruning and an ID range index

3. **Tags Not Decoded**: DuckDB `Map` values are not deserialized yet
   - `comment` tags are synthesized from `description` in the formatters

### Future Enhancements

- [ ] Connection pooling for better performance
- [ ] Query result caching
- [ ] Streaming results for large queries
- [ ] Partition pruning and metadata caching
- [ ] Geographic bounding box queries
- [ ] Advanced aggregations (user statistics, temporal analysis)
- [ ] Index building for common queries

## Examples

See `api/query/engine.rs` and `api/query/partition.rs` for the current implementation.
