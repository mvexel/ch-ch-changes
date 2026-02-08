"""Metadata management for dataset statistics (metadata.json)."""

from __future__ import annotations

import json
import logging
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from ch_ch_changes.models import BatchStats, DatasetMetadata, PartitionMetadata

logger = logging.getLogger(__name__)


def load_metadata(metadata_path: Path) -> Optional[DatasetMetadata]:
    """Load metadata from a JSON file, returning None if not found."""
    if not metadata_path.exists():
        return None

    data = json.loads(metadata_path.read_text())
    return _dict_to_metadata(data)


def find_metadata_file(data_path: Path) -> Optional[Path]:
    """Find metadata.json in standard locations."""
    candidates = [
        data_path / "delta" / "metadata.json",
        data_path / "base" / "metadata.json",
        data_path / "metadata.json",
    ]
    for p in candidates:
        if p.exists():
            return p
    return None


def get_metadata_path(data_path: Path) -> Path:
    """Get the preferred metadata path (prefers delta/ if it exists)."""
    delta_dir = data_path / "delta"
    if delta_dir.exists():
        return delta_dir / "metadata.json"
    return data_path / "metadata.json"


def update_metadata_incremental(metadata_path: Path, batch_stats: BatchStats) -> None:
    """Update metadata.json incrementally with new batch statistics."""
    metadata = load_metadata(metadata_path)
    if metadata is None:
        metadata = DatasetMetadata(
            version="1.0",
            generated_at=datetime.now(timezone.utc).isoformat(),
        )

    # Build partition map from existing
    partition_map: dict[tuple[int, int], PartitionMetadata] = {}
    for p in metadata.partitions:
        partition_map[(p.year, p.month)] = p

    # Merge batch stats
    for key, count in batch_stats.partition_counts.items():
        year, month = key
        min_id = batch_stats.partition_min_ids.get(key, 2**63 - 1)
        max_id = batch_stats.partition_max_ids.get(key, -(2**63))
        min_created_at = batch_stats.partition_min_created_at.get(key)
        max_created_at = batch_stats.partition_max_created_at.get(key)

        if key in partition_map:
            p = partition_map[key]
            p.count += count
            # Handle None values for min_id/max_id (base layer may not have these)
            if p.min_id is not None and min_id is not None:
                p.min_id = min(p.min_id, min_id)
            elif min_id is not None:
                p.min_id = min_id

            if p.max_id is not None and max_id is not None:
                p.max_id = max(p.max_id, max_id)
            elif max_id is not None:
                p.max_id = max_id

            if min_created_at is not None:
                p.min_created_at = (
                    min(p.min_created_at, min_created_at)
                    if p.min_created_at is not None
                    else min_created_at
                )
            if max_created_at is not None:
                p.max_created_at = (
                    max(p.max_created_at, max_created_at)
                    if p.max_created_at is not None
                    else max_created_at
                )
        else:
            partition_map[key] = PartitionMetadata(
                year=year,
                month=month,
                path=f"year={year}/month={month:02d}",
                count=count,
                min_id=min_id,
                max_id=max_id,
                min_created_at=min_created_at,
                max_created_at=max_created_at,
            )

    partitions = sorted(partition_map.values(), key=lambda p: (p.year, p.month))

    metadata.total_changesets += batch_stats.total_count
    metadata.partition_count = len(partitions)
    metadata.partitions = partitions

    # Recompute global bounds from partitions
    min_cats = [p.min_created_at for p in partitions if p.min_created_at is not None]
    max_cats = [p.max_created_at for p in partitions if p.max_created_at is not None]
    metadata.min_created_at = min(min_cats) if min_cats else None
    metadata.max_created_at = max(max_cats) if max_cats else None
    metadata.generated_at = datetime.now(timezone.utc).isoformat()

    _write_metadata_atomic(metadata_path, metadata)


def _write_metadata_atomic(path: Path, metadata: DatasetMetadata) -> None:
    """Write metadata atomically via temp file + rename."""
    path.parent.mkdir(parents=True, exist_ok=True)
    data = _metadata_to_dict(metadata)
    tmp_path = path.with_suffix(".json.tmp")
    with open(tmp_path, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp_path, path)


def _metadata_to_dict(metadata: DatasetMetadata) -> dict:
    return {
        "version": metadata.version,
        "generated_at": metadata.generated_at,
        "total_changesets": metadata.total_changesets,
        "partition_count": metadata.partition_count,
        "min_created_at": metadata.min_created_at,
        "max_created_at": metadata.max_created_at,
        "partitions": [
            {
                "year": p.year,
                "month": p.month,
                "path": p.path,
                "count": p.count,
                "min_id": p.min_id,
                "max_id": p.max_id,
                "min_created_at": p.min_created_at,
                "max_created_at": p.max_created_at,
            }
            for p in metadata.partitions
        ],
    }


def _dict_to_metadata(data: dict) -> DatasetMetadata:
    return DatasetMetadata(
        version=data.get("version", "1.0"),
        generated_at=data.get("generated_at", ""),
        total_changesets=data.get("total_changesets", 0),
        partition_count=data.get("partition_count", 0),
        min_created_at=data.get("min_created_at"),
        max_created_at=data.get("max_created_at"),
        partitions=[
            PartitionMetadata(
                year=p["year"],
                month=p["month"],
                path=p["path"],
                count=p["count"],
                min_id=p.get("min_id"),
                max_id=p.get("max_id"),
                min_created_at=p.get("min_created_at"),
                max_created_at=p.get("max_created_at"),
            )
            for p in data.get("partitions", [])
        ],
    )
