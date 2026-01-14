# Changeset Replication Plan

## Goal

Seamlessly serve changeset data, including recent updates, by transparently merging a weekly base snapshot with a continuously updated delta dataset. The client should not need to know about the underlying data separation or request "live" data explicitly.

## Architectural Choices

### Base + Delta Datasets

- **Base snapshot**: Weekly rebuild from the full OSM changeset dump into partitioned Parquet (`year=/month=`).
- **Delta dataset**: Append-only Parquet files derived from replication diffs, partitioned by `created_at` (`year=/month=/day=`).
- **Unified Schema**: Delta Parquet files use the **exact same schema** as Base files. No extra metadata columns are added.

### Unified Query Strategy

- **Transparent Access**: The Query Engine reads from both `base/` and `delta/` directories for all requests.
- **Deduplication ("Latest Wins")**:
    - Queries reading from potentially overlapping time ranges (where a changeset might exist in both Base and Delta, or multiple times in Delta) will apply a deduplication rule.
    - Rule: If multiple records exist for the same `id`, prefer the one with `closed_at` set (implies completed), or the one with the latest timestamp.
    - Implementation: Use SQL `QUALIFY` or window functions (e.g., `ROW_NUMBER() OVER (PARTITION BY id ORDER BY closed_at DESC NULLS LAST) = 1`).

### Weekly Rebootstrap

To keep query performance high and storage tidy:
1.  **Weekly Job**: Re-download the full planet dump and rebuild `base/`.
2.  **Reset Delta**: Once the new base is swapped in, archive/delete the old `delta/` files that are now covered by the new base.
3.  **Resume**: Start fetching replication diffs from the timestamp of the new base snapshot.

## Data Layout (On Disk)

```
data/
  base/
    year=YYYY/
      month=MM/
        *.parquet
  delta/
    year=YYYY/
      month=MM/
        day=DD/
          *.parquet
  indexes/
    partition_stats.parquet
    user_stats.parquet
```

## Replication Workflow

1.  **Ingestion Worker**:
    - Identify the timestamp of the latest data in `base/`.
    - Fetch OSM replication diffs (XML) starting from that timestamp.
    - Parse and accumulate changesets.
    - Write to `delta/` Parquet files (partitioned by day/hour).
2.  **Query Engine**:
    - On startup/query, detect partitions in both `base/` and `delta/`.
    - For a given time range query (e.g., "Jan 14th"), identify relevant partitions from both sources.
    - Execute query with deduplication logic.

## Task Breakdown

### A) Critical: Fix Tag Parsing
- **Priority**: High (Blocker)
- **Task**: Fix the `Value::Map` handling in `api/query/engine.rs`.
- **Why**: We cannot read tags from DuckDB currently. Without this, new delta data (or base data) is missing crucial metadata.

### B) Ingestion Worker
- Create a tool/script to fetch replication diffs.
- Logic: `Last Base Timestamp` -> `OSM Replication API` -> `Parse XML` -> `Write Parquet`.
- Ensure output schema matches Base exactly.

### C) Unified Query Engine
- Update `QueryEngine` to scan `delta/` directory.
- Update SQL generation to include `delta` files in the `read_parquet` list.
- Add deduplication logic (`QUALIFY` or similar) to handle overlaps.

### D) Ops & Automation
- Script the weekly re-bootstrap (download -> build base -> swap -> clean delta).

## Open Questions

- **Delta Retention**: Since we rely on the weekly dump, strictly speaking, we only *need* to keep delta files since the last dump.