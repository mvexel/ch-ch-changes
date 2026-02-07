"""Tests for XML parsing."""

import io

from ch_ch_changes.core.parsing import parse_changesets, parse_timestamp

VALID_XML = b"""\
<?xml version="1.0" encoding="UTF-8"?>
<osm>
  <changeset id="123" created_at="2024-05-01T10:30:00Z" closed_at="2024-05-01T11:00:00Z"
             open="false" user="testuser" uid="42"
             min_lat="51.0" min_lon="-1.0" max_lat="52.0" max_lon="0.0"
             num_changes="10" comments_count="2">
    <tag k="comment" v="fixing roads"/>
    <tag k="source" v="survey"/>
  </changeset>
</osm>
"""


def test_parse_valid_xml():
    result = list(parse_changesets(io.BytesIO(VALID_XML)))
    assert len(result) == 1
    cs = result[0]
    assert cs.id == 123
    assert cs.user == "testuser"
    assert cs.uid == 42
    assert cs.open is False
    assert cs.num_changes == 10
    assert cs.comments_count == 2
    assert cs.min_lat == 51.0
    assert cs.max_lon == 0.0
    assert cs.tags == {"comment": "fixing roads", "source": "survey"}
    assert cs.description == "fixing roads"  # Set from comment tag


def test_parse_comment_tag_sets_description():
    xml = b"""\
<?xml version="1.0" encoding="UTF-8"?>
<osm><changeset id="1" open="true">
  <tag k="comment" v="my description"/>
</changeset></osm>
"""
    result = list(parse_changesets(io.BytesIO(xml)))
    assert result[0].description == "my description"
    assert result[0].tags["comment"] == "my description"


def test_parse_empty_changeset():
    xml = b'<osm><changeset id="99" open="true"/></osm>'
    result = list(parse_changesets(io.BytesIO(xml)))
    assert len(result) == 1
    assert result[0].id == 99
    assert result[0].open is True
    assert result[0].tags == {}
    assert result[0].description is None


def test_parse_multiple_changesets():
    xml = b"""\
<osm>
  <changeset id="1" open="true"/>
  <changeset id="2" open="false"/>
  <changeset id="3" open="true"/>
</osm>
"""
    result = list(parse_changesets(io.BytesIO(xml)))
    assert len(result) == 3
    assert [cs.id for cs in result] == [1, 2, 3]


def test_parse_special_characters_in_tags():
    xml = b"""\
<osm><changeset id="1" open="true">
  <tag k="comment" v="roads &amp; bridges &lt;fixed&gt;"/>
</changeset></osm>
"""
    result = list(parse_changesets(io.BytesIO(xml)))
    assert result[0].tags["comment"] == "roads & bridges <fixed>"


def test_parse_empty_input():
    xml = b"<osm></osm>"
    result = list(parse_changesets(io.BytesIO(xml)))
    assert result == []


def test_parse_missing_optional_fields():
    xml = b'<osm><changeset id="5" open="true"/></osm>'
    cs = list(parse_changesets(io.BytesIO(xml)))[0]
    assert cs.created_at is None
    assert cs.closed_at is None
    assert cs.user is None
    assert cs.uid is None
    assert cs.min_lat is None
    assert cs.num_changes == 0


def test_parse_timestamp_formats():
    assert parse_timestamp("2024-05-01T10:30:00Z") is not None
    assert parse_timestamp("2024-05-01T10:30:00+00:00") is not None


def test_continue_on_error():
    # Malformed XML that would cause an error mid-stream
    xml = b"""\
<osm>
  <changeset id="1" open="true"/>
  <changeset id="INVALID_NOT_INT" open="true"/>
  <changeset id="3" open="true"/>
</osm>
"""
    result = list(parse_changesets(io.BytesIO(xml), continue_on_error=True))
    # Should get changesets 1 and 3, skipping the bad one
    ids = [cs.id for cs in result]
    assert 1 in ids
    assert 3 in ids
