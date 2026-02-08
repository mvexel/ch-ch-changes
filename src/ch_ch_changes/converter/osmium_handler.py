"""
PyOsmium handler for parsing OSM changesets.
"""
from __future__ import annotations

import logging
from typing import Any

import osmium

logger = logging.getLogger(__name__)


class ChangesetHandler(osmium.SimpleHandler):
    """Parses OSM changesets and accumulates them in a buffer."""

    def __init__(self):
        super().__init__()
        self.buffer: list[dict[str, Any]] = []

    def changeset(self, c: osmium.osm.Changeset):
        """Process a single changeset."""
        tags = {t.k: t.v for t in c.tags}
        
        # Helper to safely get timestamps (they can be None in some edge cases or closed_at not set)
        # pyosmium returns epoch (1970-01-01) for missing timestamps
        created_ts = c.created_at.timestamp() if c.created_at else 0
        created_at = created_ts * 1000 if created_ts > 0 else None
        
        closed_ts = c.closed_at.timestamp() if c.closed_at else 0
        closed_at = closed_ts * 1000 if closed_ts > 0 else None

        record = {
            "id": c.id,
            "created_at": created_at,
            "closed_at": closed_at,
            "open": c.open,
            "user": c.user,
            "uid": c.uid,
            "min_lat": c.bounds.bottom_left.lat if c.bounds.valid() else None,
            "min_lon": c.bounds.bottom_left.lon if c.bounds.valid() else None,
            "max_lat": c.bounds.top_right.lat if c.bounds.valid() else None,
            "max_lon": c.bounds.top_right.lon if c.bounds.valid() else None,
            "num_changes": c.num_changes,
            "comments_count": 0,  # Not exposed in pyosmium Changeset object currently
            "replication_sequence": None,
            "description": tags.get("comment"),
            "tags": tags,
        }
        self.buffer.append(record)

    def clear_buffer(self):
        """Clear the internal buffer."""
        self.buffer.clear()
