# ch-ch-changes

![bowie](https://chch.osm.lol/CH-CH-CHANGES.png)

*The missing API for OSM Changeset Metadata queries.*

The OpenStreetMap API [lets you query changeset metadata](https://wiki.openstreetmap.org/wiki/API_v0.6#Query:_GET_/api/0.6/changesets), but the usefulness of that API is limited by a result maximum of 100.

This API lets you query by entire days, hours, minutes, or a range of changeset IDs, and supports additional output formats like `jsonl` for efficient streaming processing.

## About Changesets

Each time an OpenStreetMap contributor uploads edits, a **changeset** is created.  
A changeset has two components:

- **Metadata** — when it was opened/closed, by whom, editor used, and contextual tags (this API)
- **Map changes** — the actual edits to OSM data

The tools / server in this project deal with the metadata only. 

## Technical Details

The system basically consists of three Rust binaries:

- `changesets-to-parquet` — archive ingestion
- `changeset-server` — API server
- `changeset-replication-worker` — minutely replication ingestion

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


### Zero-Downtime Updates

The API watches the filesystem and immediately serves new Parquet partitions as they appear.

### Trade-offs and Quirks

- Date ranges limited to 1 day
- >100k changesets rejected
- Deduplication happens at query-time
- No spatial queries or SQL interface
- Replication state files are off-by-one (handled internally)


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
