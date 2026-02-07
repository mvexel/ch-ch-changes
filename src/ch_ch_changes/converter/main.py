"""CLI tool: convert OSM changeset XML to Parquet.

NOTE: For large planet dump files (40GB+ XML), use the Rust converter instead:
    cargo run --release --bin chch-convert -- --input changesets-latest.osm.bz2 --output-dir /data/base --partition

This Python converter is suitable for:
- Small changeset XML files (< 1GB)
- Ad-hoc conversions
- Testing

The Rust converter is 5-10x faster and has lower memory usage for planet-scale data.
"""

from __future__ import annotations

import argparse
import bz2
import logging
import sys
from pathlib import Path

from ch_ch_changes.core.metadata import get_metadata_path, update_metadata_incremental
from ch_ch_changes.core.parsing import parse_changesets
from ch_ch_changes.core.writer import PartitionedWriter, write_flat

logger = logging.getLogger(__name__)


def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    parser = argparse.ArgumentParser(description="Convert OSM changeset XML to Parquet")
    parser.add_argument(
        "--input",
        "-i",
        required=True,
        help='Input XML file (or .osm.bz2). Use "-" for stdin.',
    )
    parser.add_argument(
        "--output",
        "-o",
        help="Output Parquet file (single file mode)",
    )
    parser.add_argument(
        "--output-dir",
        help="Output directory (partitioned mode)",
    )
    parser.add_argument(
        "--partition",
        action="store_true",
        help="Enable partitioned output by year/month",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=10000,
        help="Records per write batch",
    )
    parser.add_argument(
        "--continue-on-error",
        action="store_true",
        help="Save partial results on parse errors",
    )

    args = parser.parse_args()

    if args.partition:
        if not args.output_dir:
            parser.error("--output-dir is required with --partition")
        _convert_partitioned(args)
    else:
        if not args.output:
            parser.error("--output is required without --partition")
        _convert_flat(args)


def _open_input(input_path: str):
    """Open input file, handling stdin and bz2 compression."""
    if input_path == "-":
        return sys.stdin.buffer

    path = Path(input_path)
    if path.suffix == ".bz2" or path.name.endswith(".osm.bz2"):
        return bz2.open(path, "rb")

    return open(path, "rb")


def _convert_flat(args: argparse.Namespace) -> None:
    """Convert to a single flat Parquet file."""
    logger.info("Converting %s -> %s (flat)", args.input, args.output)

    with _open_input(args.input) as f:
        changesets = list(parse_changesets(f, continue_on_error=args.continue_on_error))

    logger.info("Parsed %d changesets, writing Parquet", len(changesets))
    write_flat(changesets, args.output)
    logger.info("Done: %s", args.output)


def _convert_partitioned(args: argparse.Namespace) -> None:
    """Convert to partitioned Parquet files."""
    output_dir = Path(args.output_dir)
    logger.info("Converting %s -> %s (partitioned)", args.input, output_dir)

    writer = PartitionedWriter(output_dir, batch_size=args.batch_size)
    count = 0

    with _open_input(args.input) as f:
        for cs in parse_changesets(f, continue_on_error=args.continue_on_error):
            writer.write_changeset(cs)
            count += 1
            if count % 100_000 == 0:
                logger.info("Processed %d changesets...", count)

    stats = writer.finish()
    logger.info("Wrote %d changesets to %s", count, output_dir)

    # Update metadata
    metadata_path = get_metadata_path(output_dir)
    update_metadata_incremental(metadata_path, stats)
    logger.info("Updated metadata at %s", metadata_path)


if __name__ == "__main__":
    main()
