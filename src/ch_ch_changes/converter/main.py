"""
CLI tool: convert OSM changeset XML to Parquet.
"""

from __future__ import annotations

import argparse
import logging
import shutil
import subprocess
import sys
import osmium
from pathlib import Path

from ch_ch_changes.core.metadata import get_metadata_path, update_metadata_incremental
from ch_ch_changes.core.writer import PartitionedWriter, write_flat
from ch_ch_changes.converter.osmium_handler import ChangesetHandler

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
        "--format",
        "-f",
        help="Force input format (e.g. osm, pbf). Required for stdin.",
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
        default=100000,
        help="Records per write batch",
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


def _get_input_file(input_arg: str, format_arg: str | None) -> tuple[str | osmium.io.File, subprocess.Popen | None]:
    """Return (input object for osmium, optional subprocess to wait on).

    For .bz2 inputs, attempts parallel decompression via lbzip2 or pbzip2
    by passing the pipe fd directly to osmium via /dev/fd/N.
    """
    if input_arg != "-" and not format_arg and input_arg.endswith(".bz2"):
        for cmd in ("lbzip2", "pbzip2"):
            if shutil.which(cmd):
                logger.info("Using %s for parallel bzip2 decompression", cmd)
                proc = subprocess.Popen(
                    [cmd, "-dc", input_arg],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                )
                fd_path = f"/dev/fd/{proc.stdout.fileno()}"
                return osmium.io.File(fd_path, "osm"), proc

    if input_arg == "-" or format_arg:
        fmt = format_arg or ("osm" if input_arg == "-" else "")
        return osmium.io.File(input_arg, fmt), None
    return input_arg, None


def _convert_flat(args: argparse.Namespace) -> None:
    """Convert to a single flat Parquet file."""
    logger.info("Converting %s -> %s (flat)", args.input, args.output)

    handler = ChangesetHandler()
    input_file, proc = _get_input_file(args.input, args.format)
    handler.apply_file(input_file)
    if proc:
        proc.wait()

    logger.info("Parsed %d changesets, writing Parquet", len(handler.buffer))
    write_flat(handler.buffer, args.output)
    logger.info("Done: %s", args.output)


def _convert_partitioned(args: argparse.Namespace) -> None:
    """Convert to partitioned Parquet files."""
    output_dir = Path(args.output_dir)
    logger.info("Converting %s -> %s (partitioned)", args.input, output_dir)

    writer = PartitionedWriter(output_dir, batch_size=args.batch_size)

    class StreamingChangesetHandler(ChangesetHandler):
        def __init__(self, writer: PartitionedWriter, batch_size: int):
            super().__init__()
            self.writer = writer
            self.limit = batch_size

        def changeset(self, c):
            super().changeset(c)
            if len(self.buffer) >= self.limit:
                logger.info("Buffered %d changesets; flushing...", len(self.buffer))
                self.writer.write_changesets(self.buffer)
                self.writer.flush_all()
                self.clear_buffer()

    stream_handler = StreamingChangesetHandler(writer, args.batch_size)
    input_file, proc = _get_input_file(args.input, args.format)
    stream_handler.apply_file(input_file)
    if proc:
        proc.wait()
    
    # Flush remaining
    if stream_handler.buffer:
        writer.write_changesets(stream_handler.buffer)

    stats = writer.finish()
    logger.info("Wrote total changesets to %s", output_dir)

    # Update metadata
    metadata_path = get_metadata_path(output_dir)
    update_metadata_incremental(metadata_path, stats)
    logger.info("Updated metadata at %s", metadata_path)


if __name__ == "__main__":
    main()
