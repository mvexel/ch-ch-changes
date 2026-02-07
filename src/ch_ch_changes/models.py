"""Data models for ch-ch-changes."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Changeset:
    """Represents an OpenStreetMap changeset.

    Fields match the Parquet schema exactly.
    Timestamps are milliseconds since epoch.
    """

    id: int = 0
    replication_sequence: Optional[int] = None
    created_at: Optional[int] = None  # millis since epoch
    closed_at: Optional[int] = None  # millis since epoch
    open: bool = True
    user: Optional[str] = None
    uid: Optional[int] = None
    min_lat: Optional[float] = None
    min_lon: Optional[float] = None
    max_lat: Optional[float] = None
    max_lon: Optional[float] = None
    num_changes: int = 0
    comments_count: int = 0
    description: Optional[str] = None
    tags: dict[str, str] = field(default_factory=dict)


@dataclass
class PartitionMetadata:
    """Per-partition metadata stored in metadata.json."""

    year: int
    month: int
    path: str
    count: int
    min_id: Optional[int] = None
    max_id: Optional[int] = None
    min_created_at: Optional[int] = None
    max_created_at: Optional[int] = None


@dataclass
class DatasetMetadata:
    """Dataset-level metadata stored in metadata.json."""

    version: str = "1.0"
    generated_at: str = ""
    total_changesets: int = 0
    partition_count: int = 0
    min_created_at: Optional[int] = None
    max_created_at: Optional[int] = None
    partitions: list[PartitionMetadata] = field(default_factory=list)


@dataclass
class ReplicationState:
    """Local replication state from state.yaml."""

    sequence: int
    last_run: str  # RFC3339 timestamp string


@dataclass
class BatchStats:
    """Statistics from a batch of changesets for metadata updates."""

    partition_counts: dict[tuple[int, int], int] = field(default_factory=dict)
    partition_min_ids: dict[tuple[int, int], int] = field(default_factory=dict)
    partition_max_ids: dict[tuple[int, int], int] = field(default_factory=dict)
    partition_min_created_at: dict[tuple[int, int], int] = field(default_factory=dict)
    partition_max_created_at: dict[tuple[int, int], int] = field(default_factory=dict)
    total_count: int = 0

    @classmethod
    def from_changesets(cls, changesets: list[Changeset]) -> BatchStats:
        stats = cls()
        for cs in changesets:
            if cs.created_at is None:
                continue
            from datetime import datetime, timezone

            dt = datetime.fromtimestamp(cs.created_at / 1000, tz=timezone.utc)
            key = (dt.year, dt.month)

            stats.partition_counts[key] = stats.partition_counts.get(key, 0) + 1

            if key not in stats.partition_min_ids or cs.id < stats.partition_min_ids[key]:
                stats.partition_min_ids[key] = cs.id
            if key not in stats.partition_max_ids or cs.id > stats.partition_max_ids[key]:
                stats.partition_max_ids[key] = cs.id

            if (
                key not in stats.partition_min_created_at
                or cs.created_at < stats.partition_min_created_at[key]
            ):
                stats.partition_min_created_at[key] = cs.created_at
            if (
                key not in stats.partition_max_created_at
                or cs.created_at > stats.partition_max_created_at[key]
            ):
                stats.partition_max_created_at[key] = cs.created_at

        stats.total_count = len(changesets)
        return stats
