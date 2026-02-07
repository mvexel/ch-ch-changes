# Rust Converter

**Fast bulk converter for OSM planet changeset dumps.**

This Rust binary (`chch-convert`) is optimized for converting large XML files (40GB+ planet dumps) to Parquet format. It's 5-10x faster than the Python converter.

## Why Keep Rust for Conversion?

The main ch-ch-changes system is now Python (server + worker), but we keep this Rust converter because:

- ✅ **5-10x faster** for planet dumps (2-4 hours vs 15-20 hours)
- ✅ **Lower memory usage** for large files
- ✅ **One-time operation** — only needed for initial data load
- ✅ **Battle-tested** — already processed 177M+ changesets successfully

## Installation

```bash
cd rust-converter
cargo build --release
```

The binary will be at `target/release/chch-convert`.

## Usage

### Convert Planet Dump

```bash
# Download latest dump (~40GB compressed)
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Convert to Parquet (takes 2-4 hours)
cargo run --release --bin chch-convert -- \
  --input changesets-latest.osm.bz2 \
  --output-dir /data/base \
  --partition \
  --batch-size 100000
```

### Options

```
--input <FILE>           Input XML (.osm, .osm.gz, or .osm.bz2)
--output <FILE>          Output single Parquet file (flat mode)
--output-dir <DIR>       Output directory (partitioned mode)
--partition              Enable Hive partitioning by year/month
--batch-size <N>         Changesets per batch (default: 50000)
--continue-on-error      Skip malformed changesets
```

### Read from stdin

```bash
curl -s https://example.com/changesets.osm | cargo run --release --bin chch-convert -- \
  --input - \
  --output-dir /data/base \
  --partition
```

## Output Format

Creates Hive-partitioned Parquet directories:

```
/data/base/
├── year=2005/month=04/data.parquet
├── year=2005/month=05/data.parquet
├── year=2006/month=01/data.parquet
└── ...
```

## Performance

**Planet dump (changesets-latest.osm.bz2):**
- Input: ~40GB compressed, ~180GB uncompressed
- Changesets: ~177M (as of Jan 2026)
- Time: **2-4 hours** (Rust) vs 15-20 hours (Python)
- Output: ~25GB Parquet (SNAPPY compression)

## After Conversion

Once base layer is loaded, switch to Python for everything else:

```bash
cd ..  # Back to repo root

# Start server and worker (Python)
docker-compose up -d

# Or with systemd
sudo systemctl start chch-server chch-worker
```

The Python worker will automatically detect your base layer cutoff and backfill to present.

## Architecture

```
Rust (this directory):
  ├── chch-convert      → Fast bulk converter (planet dumps)
  └── core library      → Parquet schema, XML parsing, writer

Python (main repo):
  ├── chch-server       → API server (FastAPI + DuckDB)
  ├── chch-worker       → Replication worker (minutely updates)
  └── chch-convert-py   → Small file converter (testing only)
```

## See Also

- [Main README](../README.md) — Full project documentation
- [Python server & worker](../src/ch_ch_changes/) — Runtime components
- [Docker deployment](../scripts/docker/README.md) — Deployment guide
