# Rust Converter (chch-convert)

**Fast bulk converter for OSM planet changeset dumps.**

This Rust binary is optimized for converting large XML files (40GB+ planet dumps) to Parquet format. It's 5-10x faster than the Python converter and has lower memory usage.

## When to Use This

✅ **Use Rust converter for:**
- Initial planet dump imports (changesets-latest.osm.bz2)
- Large historical XML files (> 1GB)
- Production bulk loads

❌ **Use Python converter for:**
- Small changeset XML files (< 1GB)
- Ad-hoc testing
- Development

## Installation

Requires Rust toolchain:

```bash
# Install Rust (if needed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build release binary
cargo build --release --bin chch-convert
```

## Usage

### Convert Planet Dump

```bash
# Download latest dump
wget https://planet.openstreetmap.org/planet/changesets-latest.osm.bz2

# Convert to Parquet (takes 2-4 hours for full history)
cargo run --release --bin chch-convert -- \
  --input changesets-latest.osm.bz2 \
  --output-dir /data/base \
  --partition \
  --batch-size 100000
```

### Options

```
--input <FILE>           Input XML file (.osm, .osm.gz, or .osm.bz2)
--output <FILE>          Output single Parquet file (flat mode)
--output-dir <DIR>       Output directory (partitioned mode)
--partition              Enable partitioning by year/month
--batch-size <N>         Changesets per batch (default: 50000)
--continue-on-error      Skip malformed changesets instead of aborting
```

### Read from stdin

```bash
curl -s https://example.com/changesets.osm | cargo run --release --bin chch-convert -- \
  --input - \
  --output-dir /data/base \
  --partition
```

## Output Format

Partitioned output creates Hive-style directories:

```
/data/base/
├── year=2005/
│   ├── month=04/data.parquet
│   ├── month=05/data.parquet
│   └── ...
├── year=2006/
│   └── month=*/data.parquet
└── ...
```

## Performance

**Planet dump conversion** (changesets-latest.osm.bz2):
- Size: ~40GB compressed, ~180GB uncompressed
- Changesets: ~177M (as of Jan 2026)
- Time: 2-4 hours on modern hardware
- Output: ~25GB Parquet (with SNAPPY compression)

**Comparison:**
- Rust converter: ~2.5 hours
- Python converter: ~15-20 hours (estimated)

## After Conversion

Once the base layer is loaded, use the Python worker for ongoing replication:

```bash
# Start Python worker to catch up from base layer cutoff
chch-worker
```

The worker will automatically detect the base layer's latest timestamp and backfill from there to present.

## Troubleshooting

### Out of memory

Reduce batch size:
```bash
cargo run --release --bin chch-convert -- --batch-size 25000 ...
```

### Malformed XML

Use `--continue-on-error` to skip bad changesets:
```bash
cargo run --release --bin chch-convert -- --continue-on-error ...
```

### Slow performance

Ensure you're using `--release` mode:
```bash
cargo build --release  # NOT cargo build
cargo run --release    # NOT cargo run
```

## See Also

- [Main README](../../README.md) — Project overview
- [Python converter](../../src/ch_ch_changes/converter/) — For small files
- [Worker](../../src/ch_ch_changes/worker/) — Ongoing replication
