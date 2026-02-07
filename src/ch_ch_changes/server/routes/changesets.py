"""Changeset query endpoints."""

from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timezone

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import Response

from ch_ch_changes.server.formatters import (
    CONTENT_TYPES,
    format_json,
    format_jsonl,
    format_xml,
)
from ch_ch_changes.server.query import QueryFilters

logger = logging.getLogger(__name__)

router = APIRouter(prefix="/changesets", tags=["Changesets"])

MAX_DATE_RANGE_DAYS = 1
MAX_ID_RANGE = 1_000_000
MAX_RESULT_SIZE = 100_000


def _extract_extension(s: str) -> tuple[str, str | None]:
    """Strip format extension (.json, .xml, .jsonl) from path segment."""
    for ext in (".json", ".jsonl", ".xml"):
        if s.endswith(ext):
            return s[: -len(ext)], ext[1:]
    return s, None


def _parse_filters(
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
) -> QueryFilters:
    return QueryFilters(
        username=user,
        uid=uid,
        open=open,
        limit=limit,
        offset=offset,
        order_by=order_by,
        order_direction=order_direction,
    )


def _format_response(changesets: list, fmt: str | None, check_size: bool = True) -> Response:
    if check_size and len(changesets) > MAX_RESULT_SIZE:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Result set too large",
                "details": (
                    f"Query returned {len(changesets)} changesets, "
                    f"but maximum allowed is {MAX_RESULT_SIZE}. "
                    "Please use a smaller range, add filters, or use pagination."
                ),
            },
        )

    if fmt == "json":
        return Response(
            content=format_json(changesets),
            media_type=CONTENT_TYPES["json"],
        )
    elif fmt in ("jsonl", "jsonlines"):
        return Response(
            content=format_jsonl(changesets),
            media_type=CONTENT_TYPES["jsonl"],
        )
    else:
        # Default to XML (matches Rust behavior)
        return Response(
            content=format_xml(changesets),
            media_type=CONTENT_TYPES["xml"],
        )


def _parse_date(date_str: str, label: str = "date") -> datetime:
    """Parse YYYYMMDD to datetime, raising HTTPException on failure."""
    try:
        return datetime.strptime(date_str, "%Y%m%d").replace(tzinfo=timezone.utc)
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": f"Invalid {label} format",
                "details": f"'{date_str}' is not a valid date. Expected format: YYYYMMDD",
            },
        )


async def _run_date_query(
    request: Request,
    start: datetime,
    end: datetime,
    filters: QueryFilters,
    fmt: str | None,
    check_size: bool = True,
) -> Response:
    engine = request.app.state.engine
    start_str = start.strftime("%Y-%m-%d %H:%M:%S")
    end_str = end.strftime("%Y-%m-%d %H:%M:%S")
    changesets = await asyncio.to_thread(
        engine.get_changesets_by_date_range, start_str, end_str, filters
    )
    return _format_response(changesets, fmt, check_size=check_size)


@router.get("/day/{date_str}")
async def get_changesets_by_day(
    request: Request,
    date_str: str,
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
):
    date_str, ext_fmt = _extract_extension(date_str)
    fmt = format or ext_fmt
    filters = _parse_filters(fmt, user, uid, open, limit, offset, order_by, order_direction)

    date = _parse_date(date_str)
    start = date.replace(hour=0, minute=0, second=0)
    end = date.replace(hour=23, minute=59, second=59)

    return await _run_date_query(request, start, end, filters, fmt)


@router.get("/date/from/{start_str}/to/{end_str}")
async def get_changesets_by_date_range(
    request: Request,
    start_str: str,
    end_str: str,
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
):
    end_str, ext_fmt = _extract_extension(end_str)
    fmt = format or ext_fmt
    filters = _parse_filters(fmt, user, uid, open, limit, offset, order_by, order_direction)

    start_date = _parse_date(start_str, "start date")
    end_date = _parse_date(end_str, "end date")

    start = start_date.replace(hour=0, minute=0, second=0)
    end = end_date.replace(hour=23, minute=59, second=59)

    days_diff = (end - start).days
    if days_diff > MAX_DATE_RANGE_DAYS:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Date range too large",
                "details": (
                    f"Requested {days_diff} days, but maximum allowed is "
                    f"{MAX_DATE_RANGE_DAYS} days. Please use a smaller date range."
                ),
            },
        )

    return await _run_date_query(request, start, end, filters, fmt, check_size=True)


@router.get("/day/{date_str}/hour/{hour_str}")
async def get_changesets_by_hour(
    request: Request,
    date_str: str,
    hour_str: str,
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
):
    hour_str, ext_fmt = _extract_extension(hour_str)
    fmt = format or ext_fmt
    filters = _parse_filters(fmt, user, uid, open, limit, offset, order_by, order_direction)

    date = _parse_date(date_str)
    try:
        hour = int(hour_str)
        if not 0 <= hour <= 23:
            raise ValueError
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid hour",
                "details": f"'{hour_str}' is not a valid hour. Must be 0-23",
            },
        )

    start = date.replace(hour=hour, minute=0, second=0)
    end = date.replace(hour=hour, minute=59, second=59)

    return await _run_date_query(request, start, end, filters, fmt)


@router.get("/day/{date_str}/hour/{hour_str}/minute/{minute_str}")
async def get_changesets_by_minute(
    request: Request,
    date_str: str,
    hour_str: str,
    minute_str: str,
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
):
    minute_str, ext_fmt = _extract_extension(minute_str)
    fmt = format or ext_fmt
    filters = _parse_filters(fmt, user, uid, open, limit, offset, order_by, order_direction)

    date = _parse_date(date_str)
    try:
        hour = int(hour_str)
        if not 0 <= hour <= 23:
            raise ValueError
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid hour",
                "details": f"'{hour_str}' is not a valid hour. Must be 0-23",
            },
        )
    try:
        minute = int(minute_str)
        if not 0 <= minute <= 59:
            raise ValueError
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid minute",
                "details": f"'{minute_str}' is not a valid minute. Must be 0-59",
            },
        )

    start = date.replace(hour=hour, minute=minute, second=0)
    end = date.replace(hour=hour, minute=minute, second=59)

    return await _run_date_query(request, start, end, filters, fmt)


@router.get("/id/from/{start_raw}/to/{end_raw}")
async def get_changesets_by_id_range(
    request: Request,
    start_raw: str,
    end_raw: str,
    format: str | None = None,
    user: str | None = None,
    uid: int | None = None,
    open: bool | None = None,
    limit: int | None = None,
    offset: int | None = None,
    order_by: str | None = None,
    order_direction: str | None = None,
):
    end_raw, ext_fmt = _extract_extension(end_raw)
    fmt = format or ext_fmt
    filters = _parse_filters(fmt, user, uid, open, limit, offset, order_by, order_direction)

    try:
        start_id = int(start_raw)
        if start_id <= 0:
            raise ValueError
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid start ID",
                "details": f"'{start_raw}' is not a valid changeset ID. Must be a positive integer",
            },
        )

    try:
        end_id = int(end_raw)
        if end_id <= 0:
            raise ValueError
    except ValueError:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid end ID",
                "details": f"'{end_raw}' is not a valid changeset ID. Must be a positive integer",
            },
        )

    if start_id > end_id:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "Invalid ID range",
                "details": f"start_id ({start_id}) must be less than or equal to end_id ({end_id})",
            },
        )

    range_size = end_id - start_id + 1
    if range_size > MAX_ID_RANGE:
        raise HTTPException(
            status_code=400,
            detail={
                "error": "ID range too large",
                "details": (
                    f"Requested range of {range_size} IDs, but maximum allowed is {MAX_ID_RANGE}."
                ),
            },
        )

    engine = request.app.state.engine
    changesets = await asyncio.to_thread(
        engine.get_changesets_by_id_range, start_id, end_id, filters
    )
    return _format_response(changesets, fmt, check_size=True)
