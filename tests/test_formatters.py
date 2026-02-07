"""Tests for output formatters."""

import json

from ch_ch_changes.server.formatters import format_json, format_jsonl, format_xml


def test_json_format(sample_changesets):
    result = format_json(sample_changesets)
    data = json.loads(result)

    assert data["version"] == "0.6"
    assert data["generator"] == "ch-ch-changes"
    assert data["copyright"] == "OpenStreetMap and contributors"
    assert len(data["changesets"]) == 2

    cs = data["changesets"][0]
    assert cs["id"] == 1
    assert cs["open"] is False
    assert cs["user"] == "alice"
    assert cs["changes_count"] == 5  # Note: changes_count not num_changes
    assert cs["comments_count"] == 6
    assert "tags" in cs
    assert cs["tags"]["comment"] == "first changeset"


def test_json_omits_none_fields(sample_changesets):
    result = format_json(sample_changesets)
    data = json.loads(result)
    cs = data["changesets"][1]  # bob's changeset has no closed_at
    assert "closed_at" not in cs


def test_json_comment_tag_from_description():
    """If description exists but 'comment' not in tags, add it."""
    from ch_ch_changes.models import Changeset

    cs = Changeset(id=1, open=True, description="my desc", tags={})
    result = format_json([cs])
    data = json.loads(result)
    assert data["changesets"][0]["tags"]["comment"] == "my desc"


def test_json_empty_tags_omitted():
    """Empty tags dict should not appear in output."""
    from ch_ch_changes.models import Changeset

    cs = Changeset(id=1, open=True, tags={})
    result = format_json([cs])
    data = json.loads(result)
    assert "tags" not in data["changesets"][0]


def test_json_empty_list():
    result = format_json([])
    data = json.loads(result)
    assert data["changesets"] == []


def test_jsonl_format(sample_changesets):
    result = format_jsonl(sample_changesets)
    lines = [line for line in result.strip().split("\n") if line]
    assert len(lines) == 2

    cs1 = json.loads(lines[0])
    assert cs1["id"] == 1

    cs2 = json.loads(lines[1])
    assert cs2["id"] == 2


def test_xml_format(sample_changesets):
    result = format_xml(sample_changesets)
    assert result.startswith('<?xml version="1.0"')
    assert '<osm version="0.6"' in result
    assert 'id="1"' in result
    assert 'id="2"' in result
    assert "</osm>" in result


def test_xml_escaping():
    from ch_ch_changes.models import Changeset

    cs = Changeset(
        id=1,
        open=True,
        user="test<>&\"'user",
        tags={"comment": 'roads & bridges "fixed"'},
    )
    result = format_xml([cs])
    assert "test&lt;&gt;&amp;&quot;&apos;user" in result
    assert "roads &amp; bridges &quot;fixed&quot;" in result


def test_xml_self_closing_no_tags():
    from ch_ch_changes.models import Changeset

    cs = Changeset(id=1, open=True, tags={})
    result = format_xml([cs])
    assert '<changeset id="1"' in result
    assert "/>" in result


def test_xml_comment_tag_from_description():
    from ch_ch_changes.models import Changeset

    cs = Changeset(id=1, open=True, description="my desc", tags={"source": "bing"})
    result = format_xml([cs])
    assert 'k="comment" v="my desc"' in result
    assert 'k="source" v="bing"' in result
