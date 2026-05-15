"""Health check and replication state endpoints."""

from __future__ import annotations

import asyncio
import logging
import time
from datetime import datetime, timezone
from pathlib import Path

import httpx
from fastapi import APIRouter, HTTPException, Request

logger = logging.getLogger(__name__)

router = APIRouter(tags=["Meta"])

API_VERSION = "0.1.0"
OSM_STATE_URL = "https://planet.openstreetmap.org/replication/changesets/state.yaml"


@router.get("/health")
async def health(request: Request):
    engine = request.app.state.engine
    data_path = engine.data_path

    # Count parquet files (blocking I/O)
    def _count():
        count = 0
        if data_path.exists():
            for _ in data_path.rglob("*.parquet"):
                count += 1
        return count

    partition_count = await asyncio.to_thread(_count)

    return {
        "status": "ok",
        "version": API_VERSION,
        "uptime_seconds": int(time.time() - request.app.state.started_at),
        "data_loaded": partition_count > 0,
        "partition_count": partition_count,
    }


@router.get("/meta/state")
async def get_replication_state(request: Request):
    engine = request.app.state.engine
    data_path = engine.data_path

    # Load state file
    state_path, content = await asyncio.to_thread(_load_state_file, data_path)
    if state_path is None:
        raise HTTPException(status_code=404, detail="Replication state not found")

    sequence, last_run_str, last_run_dt = _parse_state(content)

    now = datetime.now(timezone.utc)
    seconds_since_update = int((now - last_run_dt).total_seconds())

    # Dataset stats (from metadata cache — fast)
    earliest_changeset_id = engine.get_earliest_id()
    latest_changeset_id = engine.get_latest_id()
    total_changesets = engine.get_total_count()
    min_created_at, max_created_at = engine.get_created_at_bounds()
    partitions = engine.get_partition_counts()

    # OSM remote state
    osm_sequence, sequences_behind = await _fetch_osm_state(sequence)

    return {
        "sequence": sequence,
        "last_run": last_run_str,
        "seconds_since_update": seconds_since_update,
        "state_path": str(state_path),
        "earliest_changeset_id": earliest_changeset_id,
        "latest_changeset_id": latest_changeset_id,
        "total_changesets": total_changesets,
        "osm_sequence": osm_sequence,
        "sequences_behind": sequences_behind,
        "min_created_at": min_created_at,
        "max_created_at": max_created_at,
        "partition_count": len(partitions),
        "partitions": [{"year": p.year, "month": p.month, "count": p.count} for p in partitions],
    }


def _load_state_file(data_path: Path) -> tuple[Path | None, str | None]:
    candidates = [
        data_path / "delta" / "state.yaml",
        data_path / "delta" / "state.txt",
        data_path / "state.yaml",
        data_path / "state.txt",
    ]
    for path in candidates:
        if path.exists():
            return path, path.read_text()
    return None, None


def _parse_state(content: str) -> tuple[int, str, datetime]:
    """Parse state.yaml content, returning (sequence, last_run_str, last_run_dt)."""
    sequence = None
    last_run_dt = None

    for line in content.splitlines():
        line = line.strip()
        if not line or line == "---":
            continue

        if line.startswith("sequence:"):
            sequence = int(line.split(":", 1)[1].strip())
        elif line.startswith("sequenceNumber:"):
            sequence = int(line.split(":", 1)[1].strip())
        elif line.startswith("last_run:"):
            last_run_dt = _parse_timestamp(line.split(":", 1)[1].strip())
        elif line.startswith("timestamp="):
            last_run_dt = _parse_timestamp(line.split("=", 1)[1].strip())

    if sequence is None:
        raise ValueError("Missing sequence in replication state")
    if last_run_dt is None:
        raise ValueError("Missing last_run in replication state")

    last_run_str = last_run_dt.strftime("%Y-%m-%dT%H:%M:%SZ")
    return sequence, last_run_str, last_run_dt


def _parse_timestamp(value: str) -> datetime:
    """Parse various timestamp formats from state files."""
    value = value.strip().strip('"').strip("'")

    formats = [
        "%Y-%m-%d %H:%M:%S.%f %z",
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%d %H:%M:%S.%f %Z",
        "%Y-%m-%d %H:%M:%S %Z",
        "%Y-%m-%dT%H:%M:%S.%f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%d %H:%M:%S.%f",
        "%Y-%m-%d %H:%M:%S",
    ]
    for fmt in formats:
        try:
            dt = datetime.strptime(value, fmt)
            if dt.tzinfo is None:
                dt = dt.replace(tzinfo=timezone.utc)
            return dt
        except ValueError:
            continue

    # Try fromisoformat
    try:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return dt
    except Exception:
        pass

    raise ValueError(f"Cannot parse timestamp: {value}")


async def _fetch_osm_state(local_sequence: int) -> tuple[int | None, int | None]:
    """Fetch OSM's current replication state."""
    try:
        async with httpx.AsyncClient(timeout=5.0) as client:
            resp = await client.get(OSM_STATE_URL)
            resp.raise_for_status()
            for line in resp.text.splitlines():
                if line.startswith("sequenceNumber:"):
                    osm_seq = int(line.split(":", 1)[1].strip())
                    return osm_seq, osm_seq - local_sequence
    except Exception as e:
        logger.warning("Failed to fetch OSM replication state: %s", e)
    return None, None
