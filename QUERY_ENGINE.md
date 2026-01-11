# DuckDB Query Engine for Parquet Files

This document describes the `QueryEngine` - a DuckDB-based wrapper for querying OSM changeset Parquet files.

## Overview

The query engine provides a high-level interface to efficiently query changeset data stored in Parquet files organized by year/month partitions. It handles:

- Connection management to DuckDB
- Partition-aware querying for optimal performance
- Type-safe conversion of Parquet rows to Rust structs
- Common query patterns (by ID, date range, user, etc.)
- Filtering and pagination

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
use changesets_to_parquet::api::QueryEngine;

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
if let Some(changeset) = engine.get_changeset_by_id(168575829)? {
    println!("ID: {}", changeset.id);
    println!("User: {:?}", changeset.user);
}
```

### Date Range Queries

```rust
use chrono::{DateTime, Utc, TimeZone};

// Query all changesets created in May 2024
let start = Utc.with_ymd_and_hms(2024, 5, 1, 0, 0, 0).unwrap();
let end = Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap();

let changesets = engine.get_changesets_by_date_range(&start, &end)?;
println!("Found {} changesets", changesets.len());
```

### Date Range with Filters

```rust
use changesets_to_parquet::api::ChangesetFilter;

let filter = ChangesetFilter {
    user: Some("mvexel".to_string()),
    open: Some(false),  // Only closed changesets
    limit: Some(100),
    offset: Some(0),
    uid: None,
};

let changesets = engine.get_changesets_by_date_range_with_filter(
    &start,
    &end,
    filter,
)?;
```

### Query by User

```rust
// Get all changesets by username (limited to 1000)
let changesets = engine.get_changesets_by_user("mvexel")?;

// Get all changesets by user ID
let changesets = engine.get_changesets_by_uid(12345)?;
```

### Open/Closed Changesets

```rust
// Get only open changesets (first 100)
let open = engine.get_open_changesets(100)?;

// Get only closed changesets (first 100)
let closed = engine.get_closed_changesets(100)?;
```

### ID Range Queries

```rust
// Get all changesets with IDs between two values
let changesets = engine.get_changesets_by_id_range(100000, 100100)?;

// With filters
let filter = ChangesetFilter {
    user: Some("mvexel".to_string()),
    ..Default::default()
};

let changesets = engine.get_changesets_by_id_range_with_filter(
    100000,
    100100,
    filter,
)?;
```

### Custom Queries

```rust
// Execute raw SQL for advanced queries
let changesets = engine.query(
    "SELECT * FROM read_parquet('data/year=*/month=*.parquet') \
     WHERE num_changes > 1000 \
     ORDER BY created_at DESC \
     LIMIT 10"
)?;
```

### Counting

```rust
// Count total changesets
let total = engine.count_changesets()?;
println!("Total changesets: {}", total);

// Count with filters
let filter = ChangesetFilter {
    user: Some("mvexel".to_string()),
    ..Default::default()
};

let user_total = engine.count_changesets_with_filter(&filter)?;
println!("Changesets by mvexel: {}", user_total);
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
}
```

### ChangesetFilter

```rust
pub struct ChangesetFilter {
    pub user: Option<String>,      // Filter by username
    pub uid: Option<i64>,          // Filter by user ID
    pub open: Option<bool>,        // None = no filter, true = open only, false = closed only
    pub limit: Option<u32>,        // Maximum number of results
    pub offset: Option<u32>,       // Number of results to skip
}

// All fields are optional - use Default::default() for no filters
let no_filters = ChangesetFilter::default();
```

### PartitionKey

```rust
pub struct PartitionKey {
    pub year: i32,
    pub month: u32,
}

// Create from timestamp (milliseconds since epoch)
let key = PartitionKey::from_timestamp_millis(1714521600000);

// Create from DateTime
use chrono::Utc;
let dt = Utc::now();
let key = PartitionKey::from_datetime(&dt);

// Get the relative path (year=2024/month=05.parquet)
let path = key.path();
```

## Partition Management

### PartitionResolver

The `PartitionResolver` provides static helper methods for partition management:

```rust
use changesets_to_parquet::api::PartitionResolver;
use std::path::Path;

let data_dir = Path::new("./data");

// List all available partitions
let partitions = PartitionResolver::list_partitions(data_dir)?;
for pk in partitions {
    println!("Partition: {}", pk.path());
}

// Find partitions for a date range
use chrono::{DateTime, Utc, TimeZone};
let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
let end = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();

let partitions = PartitionResolver::partitions_for_date_range(
    data_dir,
    &start,
    &end,
)?;

// Build glob pattern for all partitions
let glob = PartitionResolver::glob_pattern(data_dir);
// Returns: "data/**/month=*.parquet"
```

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
   // Good - uses partitions from May only
   engine.get_changesets_by_date_range(&may_start, &may_end)?;
   ```

2. **Filter Before Pagination**: Apply filters before using limit/offset
   ```rust
   let filter = ChangesetFilter {
       user: Some("mvexel".to_string()),
       limit: Some(100),
       ..Default::default()
   };
   ```

3. **Use Custom Queries for Complex Logic**: The DuckDB query engine is powerful
   ```rust
   engine.query(
       "SELECT user, COUNT(*) as count \
        FROM read_parquet('data/year=*/month=*.parquet') \
        WHERE num_changes > 100 \
        GROUP BY user \
        ORDER BY count DESC \
        LIMIT 10"
   )?;
   ```

## Error Handling

The `QueryEngine` uses the `anyhow::Result` type for error handling:

```rust
use changesets_to_parquet::api::QueryEngine;
use anyhow::{Context, Result};

fn query_example() -> Result<()> {
    let engine = QueryEngine::new("/nonexistent/path")
        .context("Failed to create query engine")?;

    let changesets = engine.get_open_changesets(100)
        .context("Failed to query open changesets")?;

    Ok(())
}
```

## Integration with the API Server

The `QueryEngine` is designed to be used with the API server to provide HTTP endpoints:

```rust
// In an axum route handler
use changesets_to_parquet::api::QueryEngine;
use axum::extract::Path;
use serde_json::json;

async fn get_changeset_by_id(
    Path(id): Path<i64>,
) -> Result<Json<Changeset>> {
    let engine = QueryEngine::new("./data")?;

    match engine.get_changeset_by_id(id)? {
        Some(changeset) => Ok(Json(changeset)),
        None => Err(/* not found */),
    }
}
```

## Testing

The query engine includes comprehensive unit tests:

```bash
# Run all query engine tests
cargo test api::query

# Run specific test
cargo test api::query::partition::tests::test_partition_key_from_timestamp

# With output
cargo test api::query -- --nocapture
```

Current test coverage:
- `test_partition_key_from_timestamp` - Timestamp to partition key conversion
- `test_partition_key_path` - Partition path generation
- `test_partition_key_ordering` - Partition key comparison
- `test_filter_default` - Default filter behavior

## Dependencies

The query engine uses:
- `duckdb` (v0.10+) - SQL engine for Parquet files
- `chrono` - Date/time handling
- `anyhow` - Error handling

The `duckdb` crate is compiled with the `bundled` feature, so no external DuckDB installation is needed.

## Limitations and Future Work

### Current Limitations

1. **In-Memory DuckDB Connection**: Currently creates an in-memory DuckDB instance per engine
   - Could be optimized with connection pooling for repeated queries

2. **Limited Optimization**: ID-range queries scan all partitions
   - Could be improved if we had a separate ID range index

3. **String Escaping**: SQL string parameters use simple escaping
   - Parameterized queries would be more robust

### Future Enhancements

- [ ] Connection pooling for better performance
- [ ] Query result caching
- [ ] Streaming results for large queries
- [ ] Geographic bounding box queries
- [ ] Advanced aggregations (user statistics, temporal analysis)
- [ ] Index building for common queries

## Examples

See the test code in `src/api/query/engine.rs` and `src/api/query/partition.rs` for additional examples.
