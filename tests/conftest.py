"""Shared test fixtures."""

from __future__ import annotations

import duckdb
import pytest

from ch_ch_changes.config import ServerConfig
from ch_ch_changes.models import Changeset
from ch_ch_changes.server.app import create_app


@pytest.fixture
def tmp_data_dir(tmp_path):
    """Create a temp data directory with test Parquet data."""
    base_dir = tmp_path / "base" / "year=2024" / "month=05"
    base_dir.mkdir(parents=True)

    parquet_path = base_dir / "data.parquet"
    conn = duckdb.connect(":memory:")
    conn.execute("""
        CREATE TABLE changesets (
            id BIGINT,
            replication_sequence UBIGINT,
            created_at TIMESTAMP,
            closed_at TIMESTAMP,
            open BOOLEAN,
            "user" VARCHAR,
            uid BIGINT,
            min_lat DOUBLE,
            min_lon DOUBLE,
            max_lat DOUBLE,
            max_lon DOUBLE,
            num_changes UINTEGER,
            comments_count UINTEGER,
            description VARCHAR,
            tags MAP(VARCHAR, VARCHAR),
            duration_seconds BIGINT,
            bbox_area_km2 DOUBLE
        )
    """)
    conn.execute("""
        INSERT INTO changesets VALUES
            (1, 100, TIMESTAMP '2024-05-01 00:00:00', TIMESTAMP '2024-05-01 01:00:00',
             false, 'alice', 10, 1.0, 2.0, 3.0, 4.0, 5, 6, 'first changeset',
             MAP {'comment': 'first changeset', 'source': 'survey'}, 3600, 248.58),
            (2, 110, TIMESTAMP '2024-05-02 12:00:00', NULL,
             true, 'bob', 20, 5.0, 6.0, 7.0, 8.0, 9, 0, 'second changeset',
             MAP {'comment': 'second changeset'}, NULL, 248.58)
    """)
    conn.execute(f"COPY changesets TO '{parquet_path}' (FORMAT PARQUET)")
    conn.close()

    return tmp_path


@pytest.fixture
def sample_changesets():
    """Sample changeset objects for testing."""
    return [
        Changeset(
            id=1,
            replication_sequence=100,
            created_at=1714521600000,  # 2024-05-01 00:00:00 UTC
            closed_at=1714525200000,  # 2024-05-01 01:00:00 UTC
            open=False,
            user="alice",
            uid=10,
            min_lat=1.0,
            min_lon=2.0,
            max_lat=3.0,
            max_lon=4.0,
            num_changes=5,
            comments_count=6,
            description="first changeset",
            tags={"comment": "first changeset", "source": "survey"},
            duration_seconds=3600,  # 1 hour
            bbox_area_km2=248.58,  # Approximate area for 2 degree x 2 degree bbox
        ),
        Changeset(
            id=2,
            replication_sequence=110,
            created_at=1714651200000,  # 2024-05-02 12:00:00 UTC
            closed_at=None,
            open=True,
            user="bob",
            uid=20,
            min_lat=5.0,
            min_lon=6.0,
            max_lat=7.0,
            max_lon=8.0,
            num_changes=9,
            comments_count=0,
            description="second changeset",
            tags={"comment": "second changeset"},
            duration_seconds=None,  # Open changeset
            bbox_area_km2=248.58,  # Approximate area for 2 degree x 2 degree bbox
        ),
    ]


@pytest.fixture
def test_app(tmp_data_dir):
    """Create a test FastAPI app backed by temp data."""
    config = ServerConfig(DATA_PATH=str(tmp_data_dir), REQUIRE_AUTH=False, PORT=0)
    app = create_app(config)
    # Manually trigger startup
    import time

    from ch_ch_changes.server.query import QueryEngine

    app.state.engine = QueryEngine(str(tmp_data_dir))
    app.state.started_at = time.time()
    app.state.config = config
    return app


@pytest.fixture
def client(test_app):
    """FastAPI test client."""
    from fastapi.testclient import TestClient

    return TestClient(test_app)
