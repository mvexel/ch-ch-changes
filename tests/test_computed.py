"""Tests for computed field functions."""


from chchchanges.core.computed import compute_bbox_area_km2, compute_duration_seconds


class TestComputeDurationSeconds:
    """Tests for compute_duration_seconds function."""

    def test_normal_duration(self):
        """Test normal closed changeset with duration."""
        created_at = 1000000
        closed_at = 1003600  # 3600ms later = 3.6 seconds
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result == 3

    def test_longer_duration(self):
        """Test changeset with longer duration."""
        created_at = 1000000
        closed_at = 1000000 + (3600 * 1000)  # 1 hour later
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result == 3600

    def test_open_changeset(self):
        """Test open changeset returns None."""
        created_at = 1000000
        closed_at = None
        open = True
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result is None

    def test_missing_created_at(self):
        """Test missing created_at returns None."""
        created_at = None
        closed_at = 1000000
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result is None

    def test_missing_closed_at(self):
        """Test missing closed_at returns None."""
        created_at = 1000000
        closed_at = None
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result is None

    def test_zero_duration(self):
        """Test changeset with zero duration."""
        created_at = 1000000
        closed_at = 1000000
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result == 0

    def test_negative_duration_clamped(self):
        """Test that negative duration is clamped to 0."""
        created_at = 1000000
        closed_at = 999000  # Closed before created (data error)
        open = False
        result = compute_duration_seconds(created_at, closed_at, open)
        assert result == 0


class TestComputeBboxAreaKm2:
    """Tests for compute_bbox_area_km2 function."""

    def test_normal_bbox(self):
        """Test normal bounding box area calculation."""
        # Small bbox around the equator (1 degree x 1 degree)
        min_lat = 0.0
        min_lon = 0.0
        max_lat = 1.0
        max_lon = 1.0
        result = compute_bbox_area_km2(min_lat, min_lon, max_lat, max_lon)
        # 1 degree at equator is approximately 111km
        # So 1x1 degree should be roughly 111 * 111 = 12321 km²
        assert result is not None
        assert 10000 < result < 15000  # Rough approximation

    def test_point_bbox(self):
        """Test point bounding box (zero area)."""
        lat = 45.0
        lon = -122.0
        result = compute_bbox_area_km2(lat, lon, lat, lon)
        assert result == 0.0

    def test_missing_min_lat(self):
        """Test missing min_lat returns None."""
        result = compute_bbox_area_km2(None, 0.0, 1.0, 1.0)
        assert result is None

    def test_missing_min_lon(self):
        """Test missing min_lon returns None."""
        result = compute_bbox_area_km2(0.0, None, 1.0, 1.0)
        assert result is None

    def test_missing_max_lat(self):
        """Test missing max_lat returns None."""
        result = compute_bbox_area_km2(0.0, 0.0, None, 1.0)
        assert result is None

    def test_missing_max_lon(self):
        """Test missing max_lon returns None."""
        result = compute_bbox_area_km2(0.0, 0.0, 1.0, None)
        assert result is None

    def test_all_missing(self):
        """Test all coordinates missing returns None."""
        result = compute_bbox_area_km2(None, None, None, None)
        assert result is None

    def test_large_bbox(self):
        """Test large bounding box (e.g., continental scale)."""
        # Rough approximation of US west coast (10 degrees lat x 5 degrees lon)
        min_lat = 32.0
        min_lon = -125.0
        max_lat = 42.0
        max_lon = -120.0
        result = compute_bbox_area_km2(min_lat, min_lon, max_lat, max_lon)
        assert result is not None
        # Should be large (hundreds of thousands of km²)
        assert result > 100000

    def test_bbox_near_poles(self):
        """Test bounding box near poles (where longitude distance shrinks)."""
        # 1 degree x 1 degree near north pole
        min_lat = 80.0
        min_lon = 0.0
        max_lat = 81.0
        max_lon = 1.0
        result = compute_bbox_area_km2(min_lat, min_lon, max_lat, max_lon)
        assert result is not None
        # Should be much smaller than same bbox at equator
        assert result < 5000

    def test_bbox_crossing_dateline(self):
        """Test bounding box crossing the dateline."""
        # This is a limitation - the simple formula doesn't handle dateline crossing
        # But let's test that it doesn't crash
        min_lat = 50.0
        min_lon = 170.0
        max_lat = 60.0
        max_lon = -170.0  # Crosses dateline
        result = compute_bbox_area_km2(min_lat, min_lon, max_lat, max_lon)
        # The calculation will be wrong, but shouldn't crash
        assert result is not None
