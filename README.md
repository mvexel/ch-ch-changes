# chchchanges

![bowie](https://chch.osm.lol/static/chchchanges.png)

*The missing API for OSM Changeset Metadata queries.*

The OpenStreetMap API [lets you query changeset metadata](https://wiki.openstreetmap.org/wiki/API_v0.6#Query:_GET_/api/0.6/changesets), but is limited to 100 results per query.

This API lets you query by entire days, hours, minutes, or ID ranges, and supports multiple output formats (XML, JSON, JSONL) for efficient batch processing.

## Quick Start

1. download a changeset metadata archive from planet.osm.org
2. convert it to the partitioned parquet structure using the provided tool

```bash
uv run chch-convert-py -i changesets-latest.osm.bz2 --output-dir /data/base --partition
```

3. start the server and worker

```bash
docker-compose up -d
```

4. confirm it works

```
curl http://localhost:3000/health
```

See [DEPLOYMENT.md](DEPLOYMENT.md) for the complete deployment guide.

## How it works

The API uses a database-less DuckDB-based query engine, with a hive-style partitioned parquet file structure backing it. The initial data load creates the structure with the base data `base/year=2026/month=01/`. The replication worker keeps its own parquet file structure that is partitioned in the same way. DuckDB lets you specify an arbitrary list of parquet files to read from. For date ranges, it is trivially straightforward to prune the list of partitions to query. For the other query types, an index is maintained that maps the keys to
