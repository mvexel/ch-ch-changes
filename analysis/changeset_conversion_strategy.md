# OSM Changeset Conversion Strategy & Architecture Analysis

## 1. The Challenge

The current implementation uses `lxml` to parse OSM changeset XML. This is functional for small files but inefficient for planet-scale data (40GB+ XML uncompressed, often distributed as bzip2). Python's DOM/Tree parsing overhead is significant.

The goal is to convert this stream into a query-optimized Parquet archive efficiently, supporting the "database-less" architecture.

## 2. Converting Tools Landscape

I have evaluated several approaches for parsing OSM Changeset XML streams and converting them to columnar formats.

### Option A: PyOsmium (Recommended for Python compatibility)

[PyOsmium](https://osmcode.org/pyosmium/) provides Python bindings for the industry-standard C++ `libosmium`.

- **Pros**: 
    - **Speed**: Releases the Global Interpreter Lock (GIL) during parsing; significantly faster than `xml.etree` or `lxml`.
    - **Correctness**: Handles weird OSM XML edge cases correctly (timestamps, users, etc.).
    - **Integration**: Can be installed as a binary wheel (`pip install osmium`). No compiler needed.
    - **Streaming**: Event-driven `SimpleHandler` fits perfectly with a streaming writer.
- **Cons**: 
    - The "Changeset" support in `libosmium` is sometimes second-class compared to Node/Way/Relation, but it is present.

### Option B: Native Rust Tooling

Writing a small, dedicated binary using Rust.

- **Crates**: `quick-xml` (fastest XML pull parser) paired with `parquet` / `arrow` crates.
- **Pros**: 
    - **Maximum Performance**: Can easily saturate disk I/O.
    - **Single Binary**: Easy deployment (COPY binary to Docker).
    - **Type Safety**: Rust enums map well to OSM structures.
- **Cons**: 
    - Introduces a new language toolchain to the repo (unless we use a multi-stage Docker build).
    - Higher initial development effort than a Python script.

### Option C: Existing CLI Tools (`osmium-tool`, `gdal`)

- **Osmium Tool**: Standard CLI. Can convert formats (XML -> OPL -> ...).
    - *Limitation*: Does not natively output Parquet. Would require an intermediate step (e.g. `osmium export -f json | arrow-json-to-parquet`).
- **GDAL/OGR `ogr2ogr`**:
    - *Limitation*: The OSM driver is heavily optimized for *Map Features* (Nodes/Ways/Relations to Points/Lines/Polygons). It does not typically expose a "Changesets" layer out of the box without custom configuration, and usually ignores changeset metadata files completely.

## 3. Architecture Review: Parquet & Partitioning

**Is Parquet the right choice?**

The current architecture relies on `DuckDB` querying a hive-partitioned Parquet dataset (`year=YYYY/month=MM/*.parquet`).

### Critique

1.  **Access Patterns vs. Storage**:
    -   **Analytics (Time-range queries)**: Parquet is **Excellent**. Columnar storage compresses repetitive data (like `user`, `editor`) extremely well (Run-Length Encoding).
    -   **Lookups (ID queries)**: Parquet is generally **Weak** for "Point lookups" (`WHERE id = 12345`).
    -   *However*, OSM Changeset IDs are roughly monotonic with time.
    -   **Optimization**: If we sort changesets by ID within each Parquet file (which is natural if processed from a dump), generic Parquet readers (and DuckDB) will use **Row Group Statistics (Min/Max)** to skip files that don't contain the requested ID. 

2.  **Partitioning Scheme (`year/month`)**:
    -   This is a safe default.
    -   **Concern**: A strictly "Year/Month" partition might result in uneven file sizes (OSM growth over time).
    -   **Recommendation**: Maintain `year/month` for directory structure, but focus on **File Size** for the actual writes. Aim for 100MB-500MB row groups/files. DuckDB handles multiple files per partition folder seamlessly.

3.  **Schema Design**:
    -   **Tags**: The tricky part.
        -   *Approach 1*: `Map<String, String>` (Parquet supports this). Flexible, but some query engines struggle with efficient filtering on map keys. DuckDB handles it well (`tags['comment']`).
        -   *Approach 2*: Flatten common tags (`comment`, `created_by`, `source`) into their own columns. High cardinality tags go into a catch-all Map.
    -   **Recommendation**: Use a native Parquet `Map<String, String>` type for `tags`. It is the most faithful representation. Promote only the top 5-10 universal attributes (like `comment`, `created_by`) to columns if query profiling shows they are accessed in 90% of queries.

### "Pushing Back" alternatives

-   **SQLite / DuckDB file**: Better for random ID lookups (B-Tree indexes).
-   **Why stick with Parquet**: The "Database-less" goal implies we might want to swap the query engine (e.g. to ClickHouse-local, Polars, or Athena/S3) later. Parquet is the universal interchange format. An opaque SQLite file locks you into SQLite/DuckDB. Using Parquet with Min/Max stats is a viable compromise for ID lookups.

## 4. Proposed Solution: The "Efficient Converter"

I propose implementing the **Python + PyOsmium + PyArrow** solution first. It offers 90% of the performance of C++/Rust with vastly simpler maintainability for this Python-centric codebase.

**Implementation Plan:**

1.  **Input**: Stream `changesets-latest.osm.bz2` (stdin or file).
2.  **Parser**: `osmium.SimpleHandler` (C++ speed, Python glue).
3.  **Batching**: Accumulate ~100k records in memory python lists (or Arrow Arrays).
4.  **Writer**: `pyarrow.parquet.ParquetWriter` writes batches.
    -   *Crucial*: Ensure `sorting_columns=[('id', 'ascending')]` implies writing metadata so DuckDB can prune effectively.
    -   *Compression*: `ZSTD` level 3 (fast, good ratio).
5.  **Output**: Partitioned directory structure.

### Proposed Schema (Arrow)

```python
schema = pa.schema([
    ('id', pa.int64()),
    ('created_at', pa.timestamp('ms', tz='UTC')),
    ('closed_at', pa.timestamp('ms', tz='UTC')),
    ('open', pa.bool_()),
    ('user', pa.string()),
    ('uid', pa.int64()),
    ('min_lat', pa.float64()),
    ('min_lon', pa.float64()),
    ('max_lat', pa.float64()),
    ('max_lon', pa.float64()),
    ('num_changes', pa.int32()),
    ('comments_count', pa.int32()),
    ('tags', pa.map_(pa.string(), pa.string()))  # Key-Value pairs
])
```

## 5. Metadata & Secondary Indexes

To completely solve the "ID Lookup" performance gap without a running DB:
-   Generate a `file_index.parquet` during conversion.
-   Schema: `file_path | min_id | max_id | min_time | max_time`
-   The API server loads this tiny index (~few MBs) into memory/DuckDB at startup.
-   ID queries check the index first -> open specific Parquet file.
