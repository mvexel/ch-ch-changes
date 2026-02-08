"""Tests for Parquet writer."""

import pyarrow.parquet as pq

from ch_ch_changes.core.schema import CHANGESET_SCHEMA
from ch_ch_changes.core.writer import PartitionedWriter, _changesets_to_table, write_flat
from ch_ch_changes.models import Changeset


def test_write_flat(tmp_path, sample_changesets):
    output = tmp_path / "output.parquet"
    write_flat(sample_changesets, output)

    assert output.exists()
    table = pq.read_table(output)
    assert len(table) == 2
    assert table.schema.field("id").type == CHANGESET_SCHEMA.field("id").type


def test_partitioned_writer(tmp_path, sample_changesets):
    writer = PartitionedWriter(tmp_path, batch_size=100)
    for cs in sample_changesets:
        writer.write_changeset(cs)
    writer.finish()

    # Both changesets are May 2024
    partition_dir = tmp_path / "year=2024" / "month=05"
    assert partition_dir.exists()
    parquet_files = list(partition_dir.glob("*.parquet"))
    assert len(parquet_files) == 1

    table = pq.read_table(parquet_files[0])
    assert len(table) == 2


def test_partition_key_derivation(tmp_path):
    """Changesets from different months go to different partitions."""
    cs_jan = Changeset(id=1, created_at=1704067200000, open=True)  # 2024-01-01
    cs_feb = Changeset(id=2, created_at=1706745600000, open=True)  # 2024-02-01

    writer = PartitionedWriter(tmp_path, batch_size=100)
    writer.write_changeset(cs_jan)
    writer.write_changeset(cs_feb)
    writer.finish()

    assert any((tmp_path / "year=2024" / "month=01").glob("*.parquet"))
    assert any((tmp_path / "year=2024" / "month=02").glob("*.parquet"))


def test_changesets_to_table_preserves_tags(sample_changesets):
    table = _changesets_to_table(sample_changesets)
    assert "tags" in table.schema.names
    # Tags should be a map type
    tags_field = table.schema.field("tags")
    assert tags_field.type == CHANGESET_SCHEMA.field("tags").type


def test_write_single_changeset(tmp_path):
    cs = Changeset(
        id=42,
        created_at=1714521600000,
        open=False,
        num_changes=1,
        comments_count=0,
        tags={"comment": "test"},
    )
    output = tmp_path / "single.parquet"
    write_flat([cs], output)

    table = pq.read_table(output)
    assert len(table) == 1
    assert table.column("id").to_pylist() == [42]
