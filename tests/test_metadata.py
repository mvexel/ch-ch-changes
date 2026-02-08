"""Tests for metadata management."""

from ch_ch_changes.core.metadata import (
    _write_metadata_atomic,
    load_metadata,
    update_metadata_incremental,
)
from ch_ch_changes.models import BatchStats, DatasetMetadata, PartitionMetadata


def test_load_nonexistent(tmp_path):
    result = load_metadata(tmp_path / "nope.json")
    assert result is None


def test_write_and_load(tmp_path):
    path = tmp_path / "metadata.json"
    metadata = DatasetMetadata(
        version="1.0",
        generated_at="2024-01-01T00:00:00Z",
        total_changesets=100,
        partition_count=1,
        partitions=[
            PartitionMetadata(
                year=2024,
                month=1,
                path="year=2024/month=01",
                count=100,
                min_id=1,
                max_id=100,
            )
        ],
    )
    _write_metadata_atomic(path, metadata)
    assert path.exists()

    loaded = load_metadata(path)
    assert loaded is not None
    assert loaded.total_changesets == 100
    assert len(loaded.partitions) == 1
    assert loaded.partitions[0].year == 2024


def test_incremental_update_new_file(tmp_path):
    path = tmp_path / "metadata.json"
    stats = BatchStats(
        partition_counts={(2024, 5): 50},
        partition_min_ids={(2024, 5): 1},
        partition_max_ids={(2024, 5): 50},
        partition_min_created_at={(2024, 5): 1714521600000},
        partition_max_created_at={(2024, 5): 1714608000000},
        total_count=50,
    )

    update_metadata_incremental(path, stats)

    loaded = load_metadata(path)
    assert loaded is not None
    assert loaded.total_changesets == 50
    assert len(loaded.partitions) == 1
    assert loaded.partitions[0].min_id == 1
    assert loaded.partitions[0].max_id == 50


def test_incremental_update_merge(tmp_path):
    path = tmp_path / "metadata.json"

    # First batch
    stats1 = BatchStats(
        partition_counts={(2024, 5): 50},
        partition_min_ids={(2024, 5): 1},
        partition_max_ids={(2024, 5): 50},
        total_count=50,
    )
    update_metadata_incremental(path, stats1)

    # Second batch extends the same partition
    stats2 = BatchStats(
        partition_counts={(2024, 5): 30},
        partition_min_ids={(2024, 5): 51},
        partition_max_ids={(2024, 5): 80},
        total_count=30,
    )
    update_metadata_incremental(path, stats2)

    loaded = load_metadata(path)
    assert loaded.total_changesets == 80
    assert loaded.partitions[0].count == 80
    assert loaded.partitions[0].min_id == 1
    assert loaded.partitions[0].max_id == 80


def test_incremental_update_new_partition(tmp_path):
    path = tmp_path / "metadata.json"

    stats1 = BatchStats(
        partition_counts={(2024, 5): 50},
        partition_min_ids={(2024, 5): 1},
        partition_max_ids={(2024, 5): 50},
        total_count=50,
    )
    update_metadata_incremental(path, stats1)

    stats2 = BatchStats(
        partition_counts={(2024, 6): 20},
        partition_min_ids={(2024, 6): 51},
        partition_max_ids={(2024, 6): 70},
        total_count=20,
    )
    update_metadata_incremental(path, stats2)

    loaded = load_metadata(path)
    assert loaded.total_changesets == 70
    assert loaded.partition_count == 2
    assert loaded.partitions[0].year == 2024
    assert loaded.partitions[0].month == 5
    assert loaded.partitions[1].month == 6


def test_atomic_write_creates_parent(tmp_path):
    path = tmp_path / "subdir" / "metadata.json"
    metadata = DatasetMetadata(version="1.0", generated_at="now")
    _write_metadata_atomic(path, metadata)
    assert path.exists()
