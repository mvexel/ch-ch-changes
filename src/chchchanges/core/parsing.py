"""Streaming XML parser for OSM changeset files."""

from __future__ import annotations

import logging
from collections.abc import Iterator
from datetime import datetime, timezone
from typing import IO, Union

from lxml import etree

from chchchanges.core.computed import compute_bbox_area_km2, compute_duration_seconds
from chchchanges.models import Changeset

logger = logging.getLogger(__name__)


def parse_timestamp(value: str) -> int | None:
    """Parse an RFC3339-ish timestamp to milliseconds since epoch."""
    for fmt in (
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%d %H:%M:%S%z",
        "%Y-%m-%d %H:%M:%S",
    ):
        try:
            dt = datetime.strptime(value, fmt)
            if dt.tzinfo is None:
                dt = dt.replace(tzinfo=timezone.utc)
            return int(dt.timestamp() * 1000)
        except ValueError:
            continue
    return None


def parse_changesets(
    source: Union[str, IO[bytes]],
    continue_on_error: bool = False,
) -> Iterator[Changeset]:
    """Parse an OSM changeset XML stream, yielding Changeset objects.

    Args:
        source: File path or file-like object containing XML.
        continue_on_error: If True, log errors and continue parsing.
    """
    context = etree.iterparse(source, events=("end",), tag="changeset")
    count = 0
    last_id = 0

    for _, elem in context:
        try:
            cs = _parse_element(elem)
            count += 1
            last_id = cs.id
            yield cs
        except Exception:
            if continue_on_error:
                logger.warning(
                    "Parse error at changeset %d (processed %d so far)",
                    last_id,
                    count,
                    exc_info=True,
                )
            else:
                raise
        finally:
            elem.clear()
            while elem.getprevious() is not None:
                del elem.getparent()[0]


def _parse_element(elem: etree._Element) -> Changeset:
    """Parse a single <changeset> element into a Changeset."""
    cs = Changeset()

    attr = elem.attrib

    cs.id = int(attr["id"])

    if "created_at" in attr:
        cs.created_at = parse_timestamp(attr["created_at"])
    if "closed_at" in attr:
        cs.closed_at = parse_timestamp(attr["closed_at"])
    cs.open = attr.get("open", "false") == "true"
    cs.user = attr.get("user")
    if "uid" in attr:
        cs.uid = int(attr["uid"])
    if "min_lat" in attr:
        cs.min_lat = float(attr["min_lat"])
    if "min_lon" in attr:
        cs.min_lon = float(attr["min_lon"])
    if "max_lat" in attr:
        cs.max_lat = float(attr["max_lat"])
    if "max_lon" in attr:
        cs.max_lon = float(attr["max_lon"])
    if "num_changes" in attr:
        cs.num_changes = int(attr["num_changes"])
    if "comments_count" in attr:
        cs.comments_count = int(attr["comments_count"])

    # Parse nested <tag> elements
    for tag_elem in elem.iterchildren("tag"):
        k = tag_elem.get("k", "")
        v = tag_elem.get("v", "")
        cs.tags[k] = v
        if k == "comment":
            cs.description = v

    # Compute derived fields
    cs.duration_seconds = compute_duration_seconds(cs.created_at, cs.closed_at, cs.open)
    cs.bbox_area_km2 = compute_bbox_area_km2(cs.min_lat, cs.min_lon, cs.max_lat, cs.max_lon)

    return cs
