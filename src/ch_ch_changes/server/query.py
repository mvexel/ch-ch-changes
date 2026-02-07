"""DuckDB query engine with integrated partition discovery."""

from __future__ import annotations

import json
import logging
import os
import time
from pathlib import Path
from typing import Optional

import duckdb

from ch_ch_changes.core.metadata import find_metadata_file, load_metadata
from ch_ch_changes.models import Changeset, DatasetMetadata

logger = logging.getLogger(__name__)

ALLOWED_ORDER_COLUMNS = frozenset(
    ["id", "created_at", "closed_at", "user", "uid", "num_changes", "comments_count", "open"]
)

PARTITION_RESCAN_INTERVAL = 30  # seconds


class QueryFilters:
    __slots__ = ("username", "uid", "open", "limit", "offset", "order_by", "order_direction")

    def __init__(
        self,
        username: Optional[str] = None,
        uid: Optional[int] = None,
        open: Optional[bool] = None,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        order_by: Optional[str] = None,
        order_direction: Optional[str] = None,
    ):
        self.username = username
        self.uid = uid
        self.open = open
        self.limit = limit
        self.offset = offset
        self.order_by = order_by
        self.order_direction = order_direction


class PartitionCount:
    __slots__ = ("year", "month", "count")

    def __init__(self, year: int, month: int, count: int):
        self.year = year
        self.month = month
        self.count = count


class QueryEngine:
    """Executes DuckDB queries against partitioned Parquet files.

    Handles partition discovery, metadata caching, and query building.
    """

    def __init__(self, data_path: str):
        self.data_path = Path(data_path)
        self._conn = duckdb.connect(":memory:")

        # Metadata cache
        self._metadata_cache: Optional[DatasetMetadata] = None
        self._metadata_mtime: Optional[float] = None

        # Partition cache (replaces standalone PartitionResolver)
        self._partitions: dict[tuple[int, int], list[Path]] = {}
        self._partitions_last_scan: float = 0
        self._scan_partitions()

    # ── Partition discovery ──────────────────────────────────────────

    def _scan_partitions(self) -> None:
        """Scan filesystem for partitioned Parquet files."""
        partitions: dict[tuple[int, int], list[Path]] = {}
        for root in self._data_roots():
            if not root.exists():
                continue
            for pf in root.rglob("*.parquet"):
                if "indexes" in pf.parts:
                    continue
                key = _parse_partition_key(pf)
                if key is not None:
                    partitions.setdefault(key, []).append(pf)
        self._partitions = partitions
        self._partitions_last_scan = time.time()

    def _maybe_rescan(self) -> None:
        if time.time() - self._partitions_last_scan > PARTITION_RESCAN_INTERVAL:
            self._scan_partitions()

    def _data_roots(self) -> list[Path]:
        roots = []
        base = self.data_path / "base"
        delta = self.data_path / "delta"
        if base.exists():
            roots.append(base)
        if delta.exists():
            roots.append(delta)
        if not roots:
            roots.append(self.data_path)
        return roots

    def _partition_globs(self, year: int, month: int) -> list[str]:
        """Get glob patterns for a specific (year, month) partition."""
        base = self.data_path / "base"
        delta = self.data_path / "delta"
        globs = []
        rel = f"year={year}/month={month:02d}"
        for root in (base, delta):
            d = root / rel
            if d.exists():
                globs.append(str(d / "*.parquet"))
        return globs

    def _all_globs(self) -> list[str]:
        """Get glob patterns covering all data."""
        base = self.data_path / "base"
        delta = self.data_path / "delta"
        globs = []
        if base.exists():
            globs.append(str(base / "year=*/month=*/*.parquet"))
        if delta.exists():
            globs.append(str(delta / "**/*.parquet"))
        return globs

    def _globs_for_date_range(self, start_date: str, end_date: str) -> list[str]:
        """Get globs for partitions covering a date range."""
        self._maybe_rescan()
        start_ym = _parse_date_to_ym(start_date)
        end_ym = _parse_date_to_ym(end_date)
        if start_ym is None or end_ym is None:
            return self._all_globs()

        globs = []
        y, m = start_ym
        while (y, m) <= end_ym:
            globs.extend(self._partition_globs(y, m))
            m += 1
            if m > 12:
                m = 1
                y += 1
        return globs if globs else self._all_globs()

    # ── Metadata cache ───────────────────────────────────────────────

    def _load_metadata(self) -> Optional[DatasetMetadata]:
        meta_path = find_metadata_file(self.data_path)
        if meta_path is None:
            return None
        try:
            mtime = os.path.getmtime(meta_path)
        except OSError:
            return None
        if self._metadata_cache is not None and self._metadata_mtime == mtime:
            return self._metadata_cache
        metadata = load_metadata(meta_path)
        if metadata is not None:
            self._metadata_cache = metadata
            self._metadata_mtime = mtime
        return metadata

    # ── SQL building ─────────────────────────────────────────────────

    def _build_source(self, globs: list[str]) -> str:
        if len(globs) == 1:
            return f"'{_esc(globs[0])}'"
        parts = ", ".join(f"'{_esc(g)}'" for g in globs)
        return f"[{parts}]"

    def _build_query(
        self,
        source: str,
        filters: QueryFilters,
        extra_where: list[str] | None = None,
        default_order: tuple[str, str] | None = None,
    ) -> str:
        parts = [
            f"SELECT * REPLACE (to_json(tags) AS tags) "
            f"FROM read_parquet({source}, hive_partitioning=true)"
        ]

        wheres = list(extra_where or [])
        if filters.username is not None:
            wheres.append(f"\"user\" = '{_esc(filters.username)}'")
        if filters.uid is not None:
            wheres.append(f"uid = {int(filters.uid)}")
        if filters.open is not None:
            wheres.append(f"open = {str(filters.open).lower()}")

        if wheres:
            parts.append("WHERE " + " AND ".join(wheres))

        parts.append(
            "QUALIFY ROW_NUMBER() OVER "
            "(PARTITION BY id ORDER BY replication_sequence DESC NULLS LAST) = 1"
        )

        order_col, order_dir = None, "ASC"
        if filters.order_by and filters.order_by in ALLOWED_ORDER_COLUMNS:
            order_col = filters.order_by
            if filters.order_direction and filters.order_direction.upper() in ("ASC", "DESC"):
                order_dir = filters.order_direction.upper()
        elif default_order:
            order_col, order_dir = default_order
        if order_col:
            parts.append(f"ORDER BY {order_col} {order_dir}")

        if filters.limit is not None:
            parts.append(f"LIMIT {int(filters.limit)}")
        if filters.offset is not None:
            parts.append(f"OFFSET {int(filters.offset)}")

        return " ".join(parts)

    # ── Query methods ────────────────────────────────────────────────

    def get_changesets_by_date_range(
        self, start_date: str, end_date: str, filters: QueryFilters
    ) -> list[Changeset]:
        globs = self._globs_for_date_range(start_date, end_date)
        if not globs:
            return []
        source = self._build_source(globs)
        sql = self._build_query(
            source,
            filters,
            extra_where=[
                f"created_at >= CAST('{_esc(start_date)}' AS TIMESTAMP)",
                f"created_at <= CAST('{_esc(end_date)}' AS TIMESTAMP)",
            ],
            default_order=("closed_at", "ASC"),
        )
        return self._execute(sql)

    def get_changesets_by_id_range(
        self, start_id: int, end_id: int, filters: QueryFilters
    ) -> list[Changeset]:
        if start_id > end_id:
            return []

        # Partition pruning via metadata (only if min_id/max_id are available)
        metadata = self._load_metadata()
        has_id_bounds = (
            metadata is not None
            and metadata.partitions
            and all(p.min_id is not None and p.max_id is not None for p in metadata.partitions)
        )
        if has_id_bounds:
            matched = []
            for p in metadata.partitions:
                if p.max_id >= start_id and p.min_id <= end_id:
                    matched.extend(self._partition_globs(p.year, p.month))
            if not matched:
                return []
            globs = matched
        else:
            globs = self._all_globs()

        if not globs:
            return []

        source = self._build_source(globs)
        sql = self._build_query(
            source,
            filters,
            extra_where=[f"id >= {int(start_id)}", f"id <= {int(end_id)}"],
            default_order=("id", "ASC"),
        )
        return self._execute(sql)

    def get_latest_id(self) -> Optional[int]:
        metadata = self._load_metadata()
        if metadata is not None and metadata.partitions:
            max_ids = [p.max_id for p in metadata.partitions if p.max_id is not None]
            if max_ids:
                return max(max_ids)
        return self._scalar("MAX(id)")

    def get_earliest_id(self) -> Optional[int]:
        metadata = self._load_metadata()
        if metadata is not None and metadata.partitions:
            min_ids = [p.min_id for p in metadata.partitions if p.min_id is not None]
            if min_ids:
                return min(min_ids)
        return self._scalar("MIN(id)")

    def get_total_count(self) -> Optional[int]:
        metadata = self._load_metadata()
        if metadata is not None:
            return metadata.total_changesets
        return self._scalar("COUNT(DISTINCT id)")

    def get_created_at_bounds(self) -> tuple[Optional[int], Optional[int]]:
        metadata = self._load_metadata()
        if metadata is not None:
            if metadata.min_created_at is not None and metadata.max_created_at is not None:
                return (metadata.min_created_at, metadata.max_created_at)
        globs = self._all_globs()
        if not globs:
            return (None, None)
        source = self._build_source(globs)
        sql = (
            f"SELECT epoch_ms(MIN(created_at)), epoch_ms(MAX(created_at)) "
            f"FROM read_parquet({source}, hive_partitioning=true)"
        )
        try:
            row = self._conn.execute(sql).fetchone()
            if row:
                return (
                    int(row[0]) if row[0] is not None else None,
                    int(row[1]) if row[1] is not None else None,
                )
        except Exception as e:
            logger.warning("Failed to get created_at bounds: %s", e)
        return (None, None)

    def get_partition_counts(self) -> list[PartitionCount]:
        metadata = self._load_metadata()
        if metadata is not None:
            return [
                PartitionCount(year=p.year, month=p.month, count=p.count)
                for p in metadata.partitions
            ]
        return []

    # ── Execution ────────────────────────────────────────────────────

    def _scalar(self, expr: str) -> Optional[int]:
        globs = self._all_globs()
        if not globs:
            return None
        source = self._build_source(globs)
        sql = f"SELECT {expr} FROM read_parquet({source}, hive_partitioning=true)"
        try:
            row = self._conn.execute(sql).fetchone()
            if row and row[0] is not None:
                return int(row[0])
        except Exception as e:
            logger.warning("Scalar query failed: %s", e)
        return None

    def _execute(self, sql: str) -> list[Changeset]:
        try:
            result = self._conn.execute(sql)
            columns = [desc[0] for desc in result.description]
            return [_row_to_changeset(dict(zip(columns, row))) for row in result.fetchall()]
        except Exception as e:
            logger.error("Query failed: %s\nSQL: %s", e, sql)
            raise


# ── Helpers ──────────────────────────────────────────────────────────


def _esc(value: str) -> str:
    """Escape single quotes for SQL string literals."""
    return value.replace("'", "''")


def _parse_partition_key(path: Path) -> Optional[tuple[int, int]]:
    """Extract (year, month) from path like .../year=2024/month=05/data.parquet."""
    year = month = None
    for part in path.parts:
        if part.startswith("year="):
            try:
                year = int(part[5:])
            except ValueError:
                pass
        elif part.startswith("month="):
            try:
                month = int(part[6:])
            except ValueError:
                pass
    if year is not None and month is not None:
        return (year, month)
    return None


def _parse_date_to_ym(date_str: str) -> Optional[tuple[int, int]]:
    """Parse a date string to (year, month)."""
    from datetime import datetime

    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y%m%d", "%Y-%m-%d"):
        try:
            dt = datetime.strptime(date_str, fmt)
            return (dt.year, dt.month)
        except ValueError:
            continue
    # RFC3339
    try:
        dt = datetime.fromisoformat(date_str.replace("Z", "+00:00"))
        return (dt.year, dt.month)
    except Exception:
        pass
    return None


def _row_to_changeset(row: dict) -> Changeset:
    """Convert a DuckDB result row to a Changeset."""

    created_at = _to_epoch_ms(row.get("created_at"))
    closed_at = _to_epoch_ms(row.get("closed_at"))

    # Tags come as JSON string from to_json(tags)
    tags_raw = row.get("tags")
    if isinstance(tags_raw, str):
        try:
            tags = json.loads(tags_raw)
            if not isinstance(tags, dict):
                tags = {}
        except (json.JSONDecodeError, TypeError):
            tags = {}
    elif isinstance(tags_raw, dict):
        tags = tags_raw
    else:
        tags = {}

    repl_seq = row.get("replication_sequence")
    if repl_seq is not None:
        repl_seq = int(repl_seq)

    return Changeset(
        id=int(row["id"]),
        replication_sequence=repl_seq,
        created_at=created_at,
        closed_at=closed_at,
        open=bool(row.get("open", False)),
        user=row.get("user"),
        uid=int(row["uid"]) if row.get("uid") is not None else None,
        min_lat=row.get("min_lat"),
        min_lon=row.get("min_lon"),
        max_lat=row.get("max_lat"),
        max_lon=row.get("max_lon"),
        num_changes=int(row.get("num_changes", 0)),
        comments_count=int(row.get("comments_count", 0)),
        description=row.get("description"),
        tags=tags,
    )


def _to_epoch_ms(val) -> Optional[int]:
    """Convert datetime/timestamp to epoch milliseconds."""
    if val is None:
        return None
    from datetime import datetime

    if isinstance(val, datetime):
        return int(val.timestamp() * 1000)
    if isinstance(val, (int, float)):
        return int(val)
    return None
