"""Compaction of old delta Parquet files."""

from __future__ import annotations

import logging
import os
import time
from pathlib import Path

import duckdb

logger = logging.getLogger(__name__)


def compact_old_deltas(
    delta_dir: Path,
    age_hours: int = 24,
) -> int:
    """Merge old hourly delta files into single per-partition files.

    Args:
        delta_dir: Path to delta directory.
        age_hours: Only compact files older than this many hours.

    Returns:
        Number of files compacted.
    """
    cutoff = time.time() - (age_hours * 3600)
    compacted = 0

    if not delta_dir.exists():
        return 0

    # Find partition directories with multiple parquet files
    for year_dir in sorted(delta_dir.glob("year=*")):
        for month_dir in sorted(year_dir.glob("month=*")):
            parquet_files = sorted(month_dir.glob("*.parquet"))
            if len(parquet_files) <= 1:
                continue

            # Filter to only old files
            old_files = [f for f in parquet_files if os.path.getmtime(f) < cutoff]
            if len(old_files) <= 1:
                continue

            logger.info("Compacting %d files in %s", len(old_files), month_dir)

            try:
                _merge_files(old_files, month_dir / "compacted.parquet")
                for f in old_files:
                    f.unlink()
                compacted += len(old_files)
            except Exception:
                logger.exception("Failed to compact %s", month_dir)
                # Clean up partial compaction
                compacted_file = month_dir / "compacted.parquet"
                if compacted_file.exists():
                    compacted_file.unlink()

    return compacted


def _merge_files(files: list[Path], output: Path) -> None:
    """Merge multiple Parquet files using DuckDB, deduplicating by id."""
    file_list = ", ".join(f"'{f}'" for f in files)
    conn = duckdb.connect(":memory:")
    conn.execute(
        f"COPY ("
        f"  SELECT * FROM read_parquet([{file_list}], hive_partitioning=false) "
        f"  QUALIFY ROW_NUMBER() OVER "
        f"    (PARTITION BY id ORDER BY replication_sequence DESC NULLS LAST) = 1"
        f") TO '{output}' (FORMAT PARQUET, COMPRESSION SNAPPY)"
    )
    conn.close()
