"""Partitioned Parquet writer for changeset data."""

from __future__ import annotations

import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

import pyarrow as pa
import pyarrow.parquet as pq

from chchchanges.core.schema import CHANGESET_SCHEMA
from chchchanges.models import BatchStats, Changeset

logger = logging.getLogger(__name__)


class PartitionedWriter:
    """Writes changesets to partitioned Parquet files (year=YYYY/month=MM/)."""

    def __init__(self, output_dir: str | Path, batch_size: int = 10000):
        self.output_dir = Path(output_dir)
        self.batch_size = batch_size
        self._buffers: dict[tuple[int, int], list[Changeset | dict[str, Any]]] = {}
        self._counts: dict[tuple[int, int], int] = {}
        self._file_counts: dict[tuple[int, int], int] = {}
        self._written_files: list[Path] = []

    def write_changeset(self, cs: Changeset) -> None:
        key = self._partition_key(cs)
        self._buffers.setdefault(key, []).append(cs)

    def write_changesets(self, changesets: list[Changeset]) -> None:
        for cs in changesets:
            self.write_changeset(cs)

    def flush_all(self) -> None:
        """Flush all partition buffers to disk immediately."""
        for key in list(self._buffers.keys()):
            if self._buffers[key]:
                self._flush_partition(key)

    def finish(self) -> BatchStats:
        """Flush all remaining data and return stats."""
        self.flush_all()
        return self._compute_stats()

    def _partition_key(self, cs: Changeset | dict[str, Any]) -> tuple[int, int]:
        created_at = cs.get("created_at") if isinstance(cs, dict) else cs.created_at
        if created_at is not None:
            # created_at is in milliseconds
            dt = datetime.fromtimestamp(created_at / 1000, tz=timezone.utc)
            return (dt.year, dt.month)
        return (1970, 1)

    def _flush_partition(self, key: tuple[int, int]) -> None:
        changesets = self._buffers.pop(key, [])
        if not changesets:
            return

        year, month = key
        partition_dir = self.output_dir / f"year={year}" / f"month={month:02d}"
        partition_dir.mkdir(parents=True, exist_ok=True)

        file_num = self._file_counts.get(key, 0)
        path = partition_dir / f"part_{file_num:04d}.parquet"
        self._file_counts[key] = file_num + 1

        first = changesets[0]
        if isinstance(first, dict):
            table = pa.Table.from_pylist(changesets, schema=CHANGESET_SCHEMA)
        else:
            table = _changesets_to_table(changesets)

        pq.write_table(table, path, compression="snappy")

        self._counts[key] = self._counts.get(key, 0) + len(changesets)
        self._written_files.append(path)
        logger.debug("Wrote %d changesets to %s", len(changesets), path)

    def _compute_stats(self) -> BatchStats:
        stats = BatchStats()
        stats.total_count = sum(self._counts.values())
        stats.partition_counts = dict(self._counts)
        return stats


def write_flat(
    changesets: list[Changeset],
    output_path: str | Path,
) -> None:
    """Write all changesets to a single flat Parquet file."""
    table = _changesets_to_table(changesets)
    pq.write_table(table, str(output_path), compression="snappy")


def write_partitioned(
    changesets: list[Changeset],
    output_dir: str | Path,
    batch_size: int = 10000,
    filename: Optional[str] = None,
) -> BatchStats:
    """Write changesets to year/month partitioned Parquet files.

    Args:
        changesets: List of changesets to write.
        output_dir: Root output directory.
        batch_size: Not used for bulk writes (kept for API compat).
        filename: Optional custom filename (default: data.parquet).
    """
    output_dir = Path(output_dir)
    partitions: dict[tuple[int, int], list[Changeset]] = {}
    for cs in changesets:
        if cs.created_at is not None:
            dt = datetime.fromtimestamp(cs.created_at / 1000, tz=timezone.utc)
            key = (dt.year, dt.month)
        else:
            key = (1970, 1)
        partitions.setdefault(key, []).append(cs)

    fname = filename or "data.parquet"
    for (year, month), partition_cs in partitions.items():
        partition_dir = output_dir / f"year={year}" / f"month={month:02d}"
        partition_dir.mkdir(parents=True, exist_ok=True)
        path = partition_dir / fname
        table = _changesets_to_table(partition_cs)

        if path.exists():
            existing = pq.read_table(path, schema=CHANGESET_SCHEMA)
            table = pa.concat_tables([existing, table])

        pq.write_table(table, str(path), compression="snappy")

    return BatchStats.from_changesets(changesets)


def _changesets_to_table(changesets: list[Changeset]) -> pa.Table:
    """Convert a list of Changeset objects to a PyArrow Table."""
    ids = []
    repl_seqs = []
    created_ats = []
    closed_ats = []
    opens = []
    users = []
    uids = []
    min_lats = []
    min_lons = []
    max_lats = []
    max_lons = []
    num_changes_list = []
    comments_counts = []
    descriptions = []
    tags_list = []
    duration_seconds_list = []
    bbox_area_km2_list = []

    for cs in changesets:
        ids.append(cs.id)
        repl_seqs.append(cs.replication_sequence)
        created_ats.append(cs.created_at)
        closed_ats.append(cs.closed_at)
        opens.append(cs.open)
        users.append(cs.user)
        uids.append(cs.uid)
        min_lats.append(cs.min_lat)
        min_lons.append(cs.min_lon)
        max_lats.append(cs.max_lat)
        max_lons.append(cs.max_lon)
        num_changes_list.append(cs.num_changes)
        comments_counts.append(cs.comments_count)
        descriptions.append(cs.description)
        tags_list.append(list(cs.tags.items()) if cs.tags else [])
        duration_seconds_list.append(cs.duration_seconds)
        bbox_area_km2_list.append(cs.bbox_area_km2)

    arrays = [
        pa.array(ids, type=pa.int64()),
        pa.array(repl_seqs, type=pa.uint64()),
        pa.array(created_ats, type=pa.timestamp("ms")),
        pa.array(closed_ats, type=pa.timestamp("ms")),
        pa.array(opens, type=pa.bool_()),
        pa.array(users, type=pa.utf8()),
        pa.array(uids, type=pa.int64()),
        pa.array(min_lats, type=pa.float64()),
        pa.array(min_lons, type=pa.float64()),
        pa.array(max_lats, type=pa.float64()),
        pa.array(max_lons, type=pa.float64()),
        pa.array(num_changes_list, type=pa.uint32()),
        pa.array(comments_counts, type=pa.uint32()),
        pa.array(descriptions, type=pa.utf8()),
        pa.array(tags_list, type=pa.map_(pa.utf8(), pa.utf8())),
        pa.array(duration_seconds_list, type=pa.int64()),
        pa.array(bbox_area_km2_list, type=pa.float64()),
    ]

    return pa.table(
        {field.name: arr for field, arr in zip(CHANGESET_SCHEMA, arrays)},
        schema=CHANGESET_SCHEMA,
    )
