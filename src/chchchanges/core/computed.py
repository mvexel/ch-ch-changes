"""Computation functions for derived changeset fields."""

from math import asin, cos, radians, sin, sqrt


def compute_duration_seconds(
    created_at: int | None, closed_at: int | None, open: bool
) -> int | None:
    """Compute duration in seconds.

    Returns None if the changeset is open or if timestamps are missing.

    Args:
        created_at: Timestamp in milliseconds since epoch
        closed_at: Timestamp in milliseconds since epoch
        open: Whether the changeset is currently open

    Returns:
        Duration in seconds, or None if not applicable
    """
    if open or created_at is None or closed_at is None:
        return None
    return max(0, (closed_at - created_at) // 1000)


def compute_bbox_area_km2(
    min_lat: float | None,
    min_lon: float | None,
    max_lat: float | None,
    max_lon: float | None,
) -> float | None:
    """Compute bounding box area using Haversine formula.

    Returns approximate area in km². Returns None if any coordinate is missing.

    The calculation uses the Haversine formula to compute great-circle distances,
    then approximates the area as width × height. This is accurate enough for
    most changeset bboxes (which are typically small).

    Args:
        min_lat: Minimum latitude in degrees
        min_lon: Minimum longitude in degrees
        max_lat: Maximum latitude in degrees
        max_lon: Maximum longitude in degrees

    Returns:
        Area in square kilometers, or None if coordinates are incomplete
    """
    if any(x is None for x in [min_lat, min_lon, max_lat, max_lon]):
        return None

    # Handle point bboxes
    if min_lat == max_lat and min_lon == max_lon:
        return 0.0

    # Earth's radius in kilometers
    R = 6371.0

    # Convert to radians
    lat1, lon1 = radians(min_lat), radians(min_lon)
    lat2, lon2 = radians(max_lat), radians(max_lon)

    # Compute height (north-south distance)
    dlat = lat2 - lat1
    a = sin(dlat / 2) ** 2
    height_km = 2 * R * asin(sqrt(a))

    # Compute width (east-west distance) at the middle latitude
    mid_lat = (lat1 + lat2) / 2
    dlon = lon2 - lon1
    a = cos(mid_lat) ** 2 * sin(dlon / 2) ** 2
    width_km = 2 * R * asin(sqrt(a))

    # Approximate area as rectangle
    return height_km * width_km
