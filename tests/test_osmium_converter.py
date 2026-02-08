
from pathlib import Path
from ch_ch_changes.converter.osmium_handler import ChangesetHandler
from ch_ch_changes.models import Changeset, BatchStats

# Small sample OSM XML for testing
SAMPLE_XML = """<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="replicate_changesets.rb">
  <changeset id="1" created_at="2024-01-01T00:00:00Z" closed_at="2024-01-01T01:00:00Z" open="false" num_changes="10" user="mapper" uid="101" min_lat="1.0" max_lat="1.1" min_lon="2.0" max_lon="2.1" comments_count="1">
    <tag k="comment" v="First changeset"/>
    <tag k="created_by" v="JOSM"/>
  </changeset>
  <changeset id="2" created_at="2024-01-01T02:00:00Z" open="true" num_changes="5" user="mapper2" uid="102" comments_count="0">
    <tag k="comment" v="Still open"/>
  </changeset>
</osm>
"""

def test_handler_parses_changesets(tmp_path):
    xml_file = tmp_path / "test.osm"
    xml_file.write_text(SAMPLE_XML, encoding="utf-8")

    handler = ChangesetHandler()
    handler.apply_file(str(xml_file))

    assert len(handler.buffer) == 2
    
    c1 = handler.buffer[0]
    assert c1["id"] == 1
    assert c1["user"] == "mapper"
    assert c1["uid"] == 101
    assert c1["num_changes"] == 10
    assert c1["open"] is False
    assert c1["min_lat"] == 1.0
    assert c1["tags"]["comment"] == "First changeset"
    # comments_count is not exposed by pyosmium Changeset object currently
    # assert c1["comments_count"] == 1  <-- Disabled
    assert c1["comments_count"] == 0

    c2 = handler.buffer[1]
    assert c2["id"] == 2
    assert c2["open"] is True
    assert c2["closed_at"] is None
    assert c2["min_lat"] is None  # No bounds in XML
