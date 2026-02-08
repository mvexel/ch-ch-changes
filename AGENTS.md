# AI Agent Guide: Changesets to Parquet

## Mission
This project provides high-performance access to OpenStreetMap (OSM) changeset metadata. It consists of:
1.  **Converter**: Efficiently transforms raw OSM XML dumps (`.osm.bz2`) into querying-optimized Parquet files.
2.  **Replication**: A worker daemon that fetches live minutely updates from OSM and appends them as hourly delta Parquet files.
3.  **API**: A read-only HTTP server (Axum) allowing users to query this data via REST endpoints, backed by DuckDB.
