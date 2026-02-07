"""Tests for the query engine."""

from ch_ch_changes.server.query import QueryEngine, QueryFilters


def test_date_range_query(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    results = engine.get_changesets_by_date_range(
        "2024-05-01 00:00:00", "2024-05-02 23:59:59", QueryFilters()
    )
    assert len(results) == 2
    ids = {cs.id for cs in results}
    assert ids == {1, 2}


def test_date_range_single_day(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    results = engine.get_changesets_by_date_range(
        "2024-05-01 00:00:00", "2024-05-01 23:59:59", QueryFilters()
    )
    assert len(results) == 1
    assert results[0].id == 1


def test_id_range_query(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    results = engine.get_changesets_by_id_range(1, 2, QueryFilters())
    assert len(results) == 2
    assert {cs.id for cs in results} == {1, 2}


def test_id_range_inverted(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    results = engine.get_changesets_by_id_range(10, 1, QueryFilters())
    assert results == []


def test_filter_by_user(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    filters = QueryFilters(username="alice")
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert all(cs.user == "alice" for cs in results)
    assert len(results) == 1


def test_filter_by_uid(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    filters = QueryFilters(uid=20)
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert len(results) == 1
    assert results[0].user == "bob"


def test_filter_by_open(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    filters = QueryFilters(open=True)
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert all(cs.open for cs in results)


def test_limit_and_offset(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    filters = QueryFilters(limit=1)
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert len(results) == 1

    filters = QueryFilters(limit=1, offset=1)
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert len(results) == 1


def test_order_by(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    filters = QueryFilters(order_by="id", order_direction="DESC")
    results = engine.get_changesets_by_id_range(1, 100, filters)
    assert results[0].id > results[-1].id


def test_get_latest_id(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    assert engine.get_latest_id() == 2


def test_get_earliest_id(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    assert engine.get_earliest_id() == 1


def test_get_total_count(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    assert engine.get_total_count() == 2


def test_tags_parsed(tmp_data_dir):
    engine = QueryEngine(str(tmp_data_dir))
    results = engine.get_changesets_by_id_range(1, 1, QueryFilters())
    assert len(results) == 1
    cs = results[0]
    assert isinstance(cs.tags, dict)
    assert "comment" in cs.tags


def test_empty_data_dir(tmp_path):
    engine = QueryEngine(str(tmp_path))
    assert engine.get_latest_id() is None
    assert engine.get_total_count() is None
    results = engine.get_changesets_by_id_range(1, 100, QueryFilters())
    assert results == []
