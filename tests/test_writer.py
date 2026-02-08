"""Tests for Parquet writer."""

import pyarrow.parquet as pq

from chchchanges.core.schema import CHANGESET_SCHEMA
from chchchanges.core.writer import PartitionedWriter, _changesets_to_table, write_flat
from chchchanges.models import Changeset


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


def test_schema_includes_computed_columns():
    """Test that the schema includes computed columns."""
    assert "duration_seconds" in CHANGESET_SCHEMA.names
    assert "bbox_area_km2" in CHANGESET_SCHEMA.names


def test_write_preserves_computed_columns(tmp_path):
    """Test that computed columns are written to Parquet."""
    cs = Changeset(
        id=42,
        created_at=1714521600000,
        closed_at=1714525200000,  # 1 hour later
        open=False,
        min_lat=0.0,
        min_lon=0.0,
        max_lat=1.0,
        max_lon=1.0,
        num_changes=1,
        comments_count=0,
        tags={},
        duration_seconds=3600,
        bbox_area_km2=12345.67,
    )
    output = tmp_path / "computed.parquet"
    write_flat([cs], output)

    table = pq.read_table(output)
    assert "duration_seconds" in table.schema.names
    assert "bbox_area_km2" in table.schema.names
    assert table.column("duration_seconds").to_pylist() == [3600]
    assert table.column("bbox_area_km2").to_pylist() == [12345.67]


def test_write_handles_null_computed_columns(tmp_path):
    """Test that null computed columns are handled correctly."""
    cs = Changeset(
        id=42,
        created_at=1714521600000,
        open=True,  # Open, so duration should be None
        num_changes=1,
        comments_count=0,
        tags={},
        duration_seconds=None,
        bbox_area_km2=None,
    )
    output = tmp_path / "null_computed.parquet"
    write_flat([cs], output)

    table = pq.read_table(output)
    assert table.column("duration_seconds").to_pylist() == [None]
    assert table.column("bbox_area_km2").to_pylist() == [None]
