# Architecture

## Data Model

This project serves changeset metadata as an append-only log of Parquet files:
- Base files are weekly snapshots (`base/year=YYYY/month=MM.parquet`).
- Delta files are hourly batches (`delta/year=YYYY/month=MM/YYYYMMDDHH.parquet`).
- Each row includes `replication_sequence`; higher values are newer. Base rows use NULL.

## Query Strategy

Queries scan both base and delta partitions for the requested date range. Deduplication happens at query time:

```
QUALIFY ROW_NUMBER() OVER (
  PARTITION BY id
  ORDER BY replication_sequence DESC NULLS LAST
) = 1
```

This ensures the latest version of each changeset is returned, regardless of which file it appears in.

## Partition Resolution

There is no partition stats index. The filesystem layout is the only index:
1. Map requested date ranges to (year, month).
2. Glob matching base and delta files.
3. Feed the file list to DuckDB.

## Replication Workflow

1. Fetch replication diffs from the OSM minutely endpoint.
2. Buffer sequences until the hour changes (or a safety threshold is reached).
3. Write an hourly Parquet batch to `delta/`.
4. Advance the replication state only after a successful batch write.

## Maintenance

Weekly re-bootstrap keeps file counts bounded:
1. Rebuild base from the full dump.
2. Remove covered delta files.
3. Restart replication from the new base timestamp.
