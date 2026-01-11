# DuckDB Query Examples

This document contains example SQL queries that can be run directly against the Parquet files using DuckDB. The API server uses the same storage layout, but it does not expose a generic SQL endpoint.

## Setup

### Using DuckDB CLI

```bash
# Install DuckDB CLI
# On macOS: brew install duckdb
# Then start the CLI:
duckdb
```

### From Within DuckDB

```sql
-- Point to your parquet files
SELECT * FROM read_parquet('path/to/data/year=*/month=*.parquet', hive_partitioning=true) LIMIT 10;
```

If you do not need the `year`/`month` partition columns, you can omit `hive_partitioning=true` and DuckDB will still read the data.

### API vs SQL

The API server exposes specific endpoints described in `HISTORIC_API_PLAN.md` and does not accept raw SQL. Use the queries in this document for DuckDB CLI exploration or offline analysis of the Parquet files.

## Basic Queries

### Count all changesets
```sql
SELECT COUNT(*) as total_changesets
FROM read_parquet('data/year=*/month=*.parquet', hive_partitioning=true);
```

### Get a specific changeset by ID
```sql
SELECT *
FROM read_parquet('data/year=*/month=*.parquet', hive_partitioning=true)
WHERE id = 168575829;
```

### List all changesets from a specific user
```sql
SELECT id, created_at, closed_at, num_changes, description
FROM read_parquet('data/year=*/month=*.parquet', hive_partitioning=true)
WHERE user = 'mvexel'
ORDER BY created_at DESC
LIMIT 100;
```

### Get all changesets by user ID
```sql
SELECT id, created_at, user, num_changes
FROM read_parquet('data/year=*/month=*.parquet', hive_partitioning=true)
WHERE uid = 12345
ORDER BY created_at DESC;
```

### Find open changesets
```sql
SELECT id, user, uid, created_at
FROM read_parquet('data/year=*/month=*.parquet')
WHERE open = true
ORDER BY created_at DESC
LIMIT 50;
```

### Find closed changesets with many changes
```sql
SELECT id, user, uid, num_changes, created_at, closed_at
FROM read_parquet('data/year=*/month=*.parquet')
WHERE open = false AND num_changes > 1000
ORDER BY num_changes DESC
LIMIT 20;
```

## Date-Based Queries

### Changesets created in a specific month
```sql
SELECT id, user, created_at, num_changes
FROM read_parquet('data/year=2024/month=05.parquet')
ORDER BY created_at DESC;
```

### Changesets from a date range
```sql
SELECT id, user, created_at, num_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at >= '2024-01-01T00:00:00Z'::TIMESTAMP
  AND created_at < '2024-02-01T00:00:00Z'::TIMESTAMP
ORDER BY created_at DESC;
```

### Changesets closed in the last 30 days
```sql
SELECT id, user, created_at, closed_at, num_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE closed_at >= current_timestamp - INTERVAL '30 days'
ORDER BY closed_at DESC
LIMIT 100;
```

### Find changesets by creation year
```sql
SELECT
    YEAR(created_at) as year,
    COUNT(*) as count
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at IS NOT NULL
GROUP BY YEAR(created_at)
ORDER BY year DESC;
```

## User Analytics

### Top 10 users by number of changesets
```sql
SELECT user, uid, COUNT(*) as changeset_count
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IS NOT NULL AND uid IS NOT NULL
GROUP BY user, uid
ORDER BY changeset_count DESC
LIMIT 10;
```

### Top 10 users by total changes made
```sql
SELECT user, uid, COUNT(*) as changesets, SUM(num_changes) as total_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IS NOT NULL AND uid IS NOT NULL
GROUP BY user, uid
ORDER BY total_changes DESC
LIMIT 10;
```

### Users with most discussion comments
```sql
SELECT
    user, uid,
    COUNT(*) as changesets,
    SUM(comments_count) as total_comments,
    ROUND(AVG(comments_count)::NUMERIC, 2) as avg_comments_per_changeset
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IS NOT NULL AND uid IS NOT NULL
GROUP BY user, uid
HAVING SUM(comments_count) > 0
ORDER BY total_comments DESC
LIMIT 20;
```

### User statistics with first and last activity
```sql
SELECT
    user, uid,
    COUNT(*) as changeset_count,
    MIN(created_at) as first_changeset,
    MAX(created_at) as last_changeset,
    SUM(num_changes) as total_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IS NOT NULL AND uid IS NOT NULL
GROUP BY user, uid
ORDER BY changeset_count DESC
LIMIT 50;
```

## Change Statistics

### Changesets by number of changes (distribution)
```sql
SELECT
    CASE
        WHEN num_changes = 0 THEN '0'
        WHEN num_changes <= 10 THEN '1-10'
        WHEN num_changes <= 100 THEN '11-100'
        WHEN num_changes <= 1000 THEN '101-1000'
        ELSE '1000+'
    END as change_range,
    COUNT(*) as count
FROM read_parquet('data/year=*/month=*.parquet')
GROUP BY change_range
ORDER BY
    CASE change_range
        WHEN '0' THEN 1
        WHEN '1-10' THEN 2
        WHEN '11-100' THEN 3
        WHEN '101-1000' THEN 4
        WHEN '1000+' THEN 5
    END;
```

### Top changesets by number of changes
```sql
SELECT id, user, created_at, num_changes, comments_count, description
FROM read_parquet('data/year=*/month=*.parquet')
ORDER BY num_changes DESC
LIMIT 20;
```

### Average changes per changeset over time
```sql
SELECT
    YEAR(created_at) as year,
    MONTH(created_at) as month,
    COUNT(*) as changeset_count,
    ROUND(AVG(num_changes)::NUMERIC, 2) as avg_changes,
    SUM(num_changes) as total_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at IS NOT NULL
GROUP BY YEAR(created_at), MONTH(created_at)
ORDER BY year DESC, month DESC
LIMIT 24;  -- Last 24 months
```

## Geographic Queries

### Changesets covering specific area (bounding box)
```sql
SELECT id, user, created_at, num_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE min_lon >= -122.5 AND max_lon <= -122.0
  AND min_lat >= 37.7 AND max_lat <= 37.9
ORDER BY created_at DESC
LIMIT 50;
```

### Changesets with widest geographic extent
```sql
SELECT
    id, user, created_at,
    (max_lat - min_lat) as lat_range,
    (max_lon - min_lon) as lon_range,
    (max_lat - min_lat) * (max_lon - min_lon) as area
FROM read_parquet('data/year=*/month=*.parquet')
WHERE min_lat IS NOT NULL AND max_lat IS NOT NULL
  AND min_lon IS NOT NULL AND max_lon IS NOT NULL
ORDER BY area DESC
LIMIT 20;
```

### Changesets by geographic region (rough grid)
```sql
SELECT
    FLOOR(min_lat * 10) / 10 as lat_bin,
    FLOOR(min_lon * 10) / 10 as lon_bin,
    COUNT(*) as changeset_count,
    SUM(num_changes) as total_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE min_lat IS NOT NULL AND min_lon IS NOT NULL
GROUP BY lat_bin, lon_bin
ORDER BY changeset_count DESC
LIMIT 50;
```

## Comment Analysis

### Changesets with discussions
```sql
SELECT id, user, created_at, comments_count, description
FROM read_parquet('data/year=*/month=*.parquet')
WHERE comments_count > 0
ORDER BY comments_count DESC
LIMIT 30;
```

### Changesets with descriptions containing keywords
```sql
SELECT id, user, created_at, num_changes, description
FROM read_parquet('data/year=*/month=*.parquet')
WHERE description ILIKE '%maproulette%'
ORDER BY created_at DESC
LIMIT 100;
```

### Comment frequency by user
```sql
SELECT
    user, uid,
    COUNT(*) as changesets_with_comments,
    SUM(comments_count) as total_comments,
    ROUND(AVG(comments_count)::NUMERIC, 2) as avg_comments
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IS NOT NULL AND comments_count > 0
GROUP BY user, uid
HAVING SUM(comments_count) > 10
ORDER BY total_comments DESC
LIMIT 50;
```

## Data Quality Queries

### Changesets with missing data
```sql
SELECT
    COUNT(CASE WHEN user IS NULL THEN 1 END) as missing_user,
    COUNT(CASE WHEN uid IS NULL THEN 1 END) as missing_uid,
    COUNT(CASE WHEN created_at IS NULL THEN 1 END) as missing_created_at,
    COUNT(CASE WHEN min_lat IS NULL THEN 1 END) as missing_bounds,
    COUNT(*) as total
FROM read_parquet('data/year=*/month=*.parquet');
```

### Unusual changesets (zero changes, no user, etc.)
```sql
SELECT id, created_at, num_changes, user, uid
FROM read_parquet('data/year=*/month=*.parquet')
WHERE num_changes = 0 OR user IS NULL OR uid IS NULL
LIMIT 50;
```

## Temporal Analysis

### Changes per month (time series)
```sql
SELECT
    DATE_TRUNC('month', created_at) as month,
    COUNT(*) as changeset_count,
    SUM(num_changes) as total_changes,
    ROUND(AVG(num_changes)::NUMERIC, 2) as avg_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at IS NOT NULL
GROUP BY DATE_TRUNC('month', created_at)
ORDER BY month DESC;
```

### Day of week analysis
```sql
SELECT
    CASE EXTRACT(isodow FROM created_at)
        WHEN 1 THEN 'Monday'
        WHEN 2 THEN 'Tuesday'
        WHEN 3 THEN 'Wednesday'
        WHEN 4 THEN 'Thursday'
        WHEN 5 THEN 'Friday'
        WHEN 6 THEN 'Saturday'
        WHEN 7 THEN 'Sunday'
    END as day_of_week,
    COUNT(*) as changeset_count
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at IS NOT NULL
GROUP BY EXTRACT(isodow FROM created_at)
ORDER BY EXTRACT(isodow FROM created_at);
```

### Hour of day analysis
```sql
SELECT
    EXTRACT(hour FROM created_at) as hour,
    COUNT(*) as changeset_count
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at IS NOT NULL
GROUP BY EXTRACT(hour FROM created_at)
ORDER BY hour;
```

## Advanced Examples

### Find rapid edits by user (multiple changesets in short time)
```sql
WITH user_changesets AS (
    SELECT
        user, uid, id, created_at,
        LAG(created_at) OVER (PARTITION BY user ORDER BY created_at) as prev_timestamp
    FROM read_parquet('data/year=*/month=*.parquet')
    WHERE user IS NOT NULL
)
SELECT
    user, uid, COUNT(*) as rapid_edits
FROM user_changesets
WHERE prev_timestamp IS NOT NULL
  AND (created_at - prev_timestamp) < INTERVAL '1 minute'
GROUP BY user, uid
HAVING COUNT(*) > 5
ORDER BY rapid_edits DESC
LIMIT 20;
```

### Changesets with both discussions and significant changes
```sql
SELECT
    id, user, created_at,
    num_changes,
    comments_count,
    description
FROM read_parquet('data/year=*/month=*.parquet')
WHERE num_changes > 500
  AND comments_count > 0
ORDER BY comments_count DESC, num_changes DESC
LIMIT 50;
```

### Compare user activity across years
```sql
SELECT
    user,
    YEAR(created_at) as year,
    COUNT(*) as changeset_count,
    SUM(num_changes) as total_changes
FROM read_parquet('data/year=*/month=*.parquet')
WHERE user IN (
    SELECT user
    FROM read_parquet('data/year=*/month=*.parquet')
    WHERE user IS NOT NULL
    GROUP BY user
    ORDER BY COUNT(*) DESC
    LIMIT 20
)
AND user IS NOT NULL
GROUP BY user, YEAR(created_at)
ORDER BY user, year DESC;
```

## Performance Tips

### Use Partitioning
For date-based queries, use specific partition files when possible:
```sql
-- Better: queries only May 2024 data
SELECT * FROM read_parquet('data/year=2024/month=05.parquet')

-- vs.

-- Slower: must scan all partitions
SELECT * FROM read_parquet('data/year=*/month=*.parquet')
```

### Use Filtering
Apply WHERE clauses to reduce data:
```sql
-- Good: filtered before aggregation
SELECT user, COUNT(*)
FROM read_parquet('data/year=*/month=*.parquet')
WHERE created_at >= '2024-01-01'::TIMESTAMP
GROUP BY user;
```

### Limit Result Sets
Use LIMIT to avoid materializing huge result sets:
```sql
-- Good
SELECT * FROM ... LIMIT 1000;

-- Bad - may run out of memory
SELECT * FROM ... (no limit)
```

## Running Queries

### Via DuckDB CLI
```bash
duckdb
D SELECT * FROM read_parquet('data/year=*/month=*.parquet') LIMIT 5;
```

### Via SQL file
```bash
duckdb < queries.sql > results.csv
```

## See Also

- [QUERY_ENGINE.md](QUERY_ENGINE.md) - Rust API documentation
- [DuckDB Documentation](https://duckdb.org/docs/)
- [Parquet Format](https://parquet.apache.org/)
