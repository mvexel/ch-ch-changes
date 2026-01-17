# OSM Changeset to Parquet converter and Bulk changeset metadata API

There are two tools here:
1. `changesets-to-parquet` This tool converts the OSM Changeset metadata dump (.osm.bz2) to a proper parquet file you can query using DuckDB.
2. `changeset-server`: A server that exposes an API and remote parquet query access using the output of the above.

> This toolkit only deals with changeset **metadata**, not with the actual map edits the changeset represents.

## Converter

### Usage

First, download a changeset dump file from https://planet.osm.org/.

```bash
changesets-to-parquet --input changesets-latest.osm.bz2 --output changesets.parquet 
```

There are additional options for chunk size, partitioning and adding a couple of different indexes to speed up common queries. See `changesets-to-parquet --help` for more. 

You can achieve significant speed-up by using `pbzip2` for multithreaded decompression and piping that into the tool.

## Server

Run with 

```bash
DATA_PATH=/opt/changeset-data/changesets .changeset-server
```

### Endpoints

- `GET /health - Health check`
- `GET /changeset/{id}.json` - Single changeset
- `GET /changesets/day/{YYYYMMDD}` - All changesets on a date
- `GET /changesets/day/{YYYYMMDD}/hour/{HH}/[minute/{MM}]` - All changesets on a date and time.
- `GET /changesets/from/{start}/to/{end}` - Date range (only handles entire days) response can be huge!
- `GET /meta/latest` - Latest changeset ID with open/close timestamps

## Geoparquet?

The only geometries in this data are bounding boxes, so most spatial operations will be cheap. If you do want geoparquet, it's straightforward to do. 
The resulting geoparquet file(s) will be ~15% bigger.

```sql
COPY (
    SELECT
        *,
        ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat) AS geometry
    FROM read_parquet('changesets.parquet')
    WHERE min_lat IS NOT NULL
) TO 'changesets_geo.parquet' (
    FORMAT PARQUET,
    CODEC 'ZSTD'
);
```

Or if you created partitioned output:

```sql
COPY (
    SELECT
        *,
        ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat) AS geometry
    FROM read_parquet('changesets/year=*/month=*.parquet')
    WHERE min_lat IS NOT NULL
) TO 'changesets_geo.parquet' (FORMAT PARQUET, CODEC 'ZSTD');
```

Or a time slice (if you used partitions to create):
```sql
COPY (
    SELECT
        *,
        ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat) AS geometry
    FROM read_parquet('changesets/year=2024/month=0[123].parquet')
    WHERE min_lat IS NOT NULL
) TO 'changesets_q1_2024_geo.parquet' (FORMAT PARQUET);
```

You can also keep the partitioning: 

```sql
COPY (
    SELECT
        *,
        ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat) AS geometry
    FROM read_parquet('changesets/year=*/month=*.parquet')
    WHERE min_lat IS NOT NULL
) TO 'geo_output' (
    FORMAT PARQUET,
    PARTITION_BY (year),
    CODEC 'ZSTD'
);
```
