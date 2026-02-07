"""Output formatters for JSON, JSONL, and XML."""

from __future__ import annotations

from datetime import datetime, timezone

from ch_ch_changes.models import Changeset

API_VERSION = "0.6"
GENERATOR = "ch-ch-changes"
COPYRIGHT = "OpenStreetMap and contributors"
ATTRIBUTION = "http://www.openstreetmap.org/copyright"
LICENSE = "http://opendatacommons.org/licenses/odbl/1-0/"


# ── JSON ─────────────────────────────────────────────────────────────


def format_json(changesets: list[Changeset], pretty: bool = False) -> str:
    """Format changesets as a JSON response with OSM metadata envelope."""
    import json

    envelope = {
        "version": API_VERSION,
        "generator": GENERATOR,
        "copyright": COPYRIGHT,
        "attribution": ATTRIBUTION,
        "license": LICENSE,
        "changesets": [_changeset_to_json_dict(cs) for cs in changesets],
    }
    if pretty:
        return json.dumps(envelope, indent=2, ensure_ascii=False)
    return json.dumps(envelope, ensure_ascii=False, separators=(",", ":"))


def _changeset_to_json_dict(cs: Changeset) -> dict:
    d: dict = {"id": cs.id}
    if cs.created_at is not None:
        d["created_at"] = _ms_to_rfc3339(cs.created_at)
    if cs.closed_at is not None:
        d["closed_at"] = _ms_to_rfc3339(cs.closed_at)
    d["open"] = cs.open
    if cs.user is not None:
        d["user"] = cs.user
    if cs.uid is not None:
        d["uid"] = cs.uid
    if cs.min_lat is not None:
        d["min_lat"] = cs.min_lat
    if cs.min_lon is not None:
        d["min_lon"] = cs.min_lon
    if cs.max_lat is not None:
        d["max_lat"] = cs.max_lat
    if cs.max_lon is not None:
        d["max_lon"] = cs.max_lon
    d["changes_count"] = cs.num_changes
    d["comments_count"] = cs.comments_count

    # Build tags dict, ensuring "comment" is present if description exists
    tags = dict(cs.tags)
    if cs.description and "comment" not in tags:
        tags["comment"] = cs.description
    if tags:
        d["tags"] = tags

    return d


# ── JSONL (NDJSON) ───────────────────────────────────────────────────


def format_jsonl(changesets: list[Changeset]) -> str:
    """Format changesets as newline-delimited JSON (one per line)."""
    import json

    lines = []
    for cs in changesets:
        line = json.dumps(_changeset_to_json_dict(cs), ensure_ascii=False, separators=(",", ":"))
        lines.append(line)
    return "\n".join(lines)


# ── XML ──────────────────────────────────────────────────────────────


def format_xml(changesets: list[Changeset]) -> str:
    """Format changesets as OSM-style XML."""
    parts = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<osm version="{API_VERSION}" generator="{GENERATOR}"'
        f' copyright="{COPYRIGHT}"'
        f' attribution="{ATTRIBUTION}"'
        f' license="{LICENSE}">',
    ]
    for cs in changesets:
        parts.append(_changeset_to_xml(cs))
    parts.append("</osm>")
    return "\n".join(parts)


def _changeset_to_xml(cs: Changeset) -> str:
    attrs = [f'id="{cs.id}"']
    if cs.created_at is not None:
        attrs.append(f'created_at="{_ms_to_rfc3339(cs.created_at)}"')
    if cs.closed_at is not None:
        attrs.append(f'closed_at="{_ms_to_rfc3339(cs.closed_at)}"')
    attrs.append(f'open="{str(cs.open).lower()}"')
    if cs.user is not None:
        attrs.append(f'user="{_xml_escape(cs.user)}"')
    if cs.uid is not None:
        attrs.append(f'uid="{cs.uid}"')
    if cs.min_lat is not None:
        attrs.append(f'min_lat="{cs.min_lat}"')
    if cs.min_lon is not None:
        attrs.append(f'min_lon="{cs.min_lon}"')
    if cs.max_lat is not None:
        attrs.append(f'max_lat="{cs.max_lat}"')
    if cs.max_lon is not None:
        attrs.append(f'max_lon="{cs.max_lon}"')
    attrs.append(f'comments_count="{cs.comments_count}"')
    attrs.append(f'changes_count="{cs.num_changes}"')

    # Build tags, ensuring "comment" is present if description exists
    tags = dict(cs.tags)
    if cs.description and "comment" not in tags:
        tags["comment"] = cs.description

    attr_str = " ".join(attrs)
    if not tags:
        return f"  <changeset {attr_str}/>"

    lines = [f"  <changeset {attr_str}>"]
    for k in sorted(tags.keys()):
        v = tags[k]
        lines.append(f'    <tag k="{_xml_escape(k)}" v="{_xml_escape(v)}"/>')
    lines.append("  </changeset>")
    return "\n".join(lines)


# ── Helpers ──────────────────────────────────────────────────────────


def _ms_to_rfc3339(ms: int) -> str:
    """Convert epoch milliseconds to RFC3339 string (seconds precision)."""
    dt = datetime.fromtimestamp(ms / 1000, tz=timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%SZ")


def _xml_escape(s: str) -> str:
    """Escape special characters for XML attribute values."""
    return (
        s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&apos;")
    )


# ── Content types ────────────────────────────────────────────────────

CONTENT_TYPES = {
    "json": "application/json",
    "jsonl": "application/x-ndjson",
    "jsonlines": "application/x-ndjson",
    "xml": "application/xml",
}
