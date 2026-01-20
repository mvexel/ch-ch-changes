# OSM Changeset to Parquet converter and Bulk changeset metadata API

This project contains a small collection of tools that function together to create an API that lets you query by entire days, hours, minutes, or a range of changeset ids, and supports additional output formats like `jsonl` for efficient streaming processing.

For end-user information, see [https://chch.osm.lol](https://chch.osm.lol).

## Architecture

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

