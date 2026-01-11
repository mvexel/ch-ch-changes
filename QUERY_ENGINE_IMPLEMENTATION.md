# QueryEngine Implementation Status

`HISTORIC_API_PLAN.md` is the reference document for this implementation. This file reflects the current state of the query engine code under `api/`.

## Location

- `api/query/engine.rs` - implemented DuckDB query wrapper
- `api/query/partition.rs` - stubbed partition resolver (not implemented yet)
- `api/query/builder.rs` - stubbed SQL builder (not implemented yet)

## Implemented Today

### QueryEngine (`api/query/engine.rs`)

Core methods:
- `new(data_path: &str)` - initialize DuckDB connection
- `get_changeset(id: i64)` - fetch a single changeset by ID
- `get_changesets_by_date_range(start: &str, end: &str, filters: &QueryFilters)`
- `get_changesets_by_id_range(start_id: i64, end_id: i64, filters: &QueryFilters)`
- `get_changesets_by_user(username: &str, filters: &QueryFilters)`
- `get_latest_id()` - max changeset ID
- `get_total_count()` - total changeset count

QueryFilters:
- `username`, `uid`, `open`, `limit`, `offset`
- builder helpers: `with_username`, `with_uid`, `with_open`, `with_limit`, `with_offset`

Behavior notes:
- Uses `read_parquet(..., hive_partitioning=true)` over `data_path/**/*.parquet`
- Date-range queries cast input strings to `TIMESTAMP` in SQL
- Parquet `created_at`/`closed_at` values are converted from DuckDB `TIMESTAMP` to epoch milliseconds
- Tag maps are not read from Parquet; formatters add `comment` from `description` (see `api/format/*`)

## Pending (per plan)

- Partition pruning and metadata in `api/query/partition.rs`
- SQL builder in `api/query/builder.rs`
- ID range indexes and partition resolution
- Metadata endpoints and caching

## Integration

The query engine is wired into the `changeset-server` binary via:
- `api/main.rs`
- `api/routes/changeset.rs`
- `api/routes/changesets.rs`

## Documentation

- `QUERY_ENGINE.md` - usage and behavior notes
- `QUERY_EXAMPLES.md` - example DuckDB queries
