"""Arrow schema definition matching the Parquet file layout."""

import pyarrow as pa

CHANGESET_SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("replication_sequence", pa.uint64(), nullable=True),
        pa.field("created_at", pa.timestamp("ms"), nullable=True),
        pa.field("closed_at", pa.timestamp("ms"), nullable=True),
        pa.field("open", pa.bool_(), nullable=False),
        pa.field("user", pa.utf8(), nullable=True),
        pa.field("uid", pa.int64(), nullable=True),
        pa.field("min_lat", pa.float64(), nullable=True),
        pa.field("min_lon", pa.float64(), nullable=True),
        pa.field("max_lat", pa.float64(), nullable=True),
        pa.field("max_lon", pa.float64(), nullable=True),
        pa.field("num_changes", pa.uint32(), nullable=False),
        pa.field("comments_count", pa.uint32(), nullable=False),
        pa.field("description", pa.utf8(), nullable=True),
        pa.field(
            "tags",
            pa.map_(pa.utf8(), pa.utf8()),
            nullable=False,
        ),
        # Computed fields
        pa.field("duration_seconds", pa.int64(), nullable=True),
        pa.field("bbox_area_km2", pa.float64(), nullable=True),
    ]
)
