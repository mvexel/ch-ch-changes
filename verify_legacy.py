
from ch_ch_changes.converter.osmium_handler import ChangesetHandler
from ch_ch_changes.core.writer import PartitionedWriter
import shutil
from pathlib import Path

# Create a minimal OSM XML that replicates "Legacy" data
# - No created_at (simulate pre-2009 or broken data)
# - No bbox
# - No user/uid
# - No tags
MINIMAL_XML = """<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
  <changeset id="999" open="false" num_changes="0"/>
</osm>
"""

def test_legacy_handling():
    input_file = Path("legacy.osm")
    input_file.write_text(MINIMAL_XML, encoding="utf-8")
    output_dir = Path("legacy_output")
    
    if output_dir.exists():
        shutil.rmtree(output_dir)

    print("Parsing legacy changeset...")
    handler = ChangesetHandler()
    handler.apply_file(str(input_file))
    
    print(f"Buffered {len(handler.buffer)} records.")
    record = handler.buffer[0]
    print(f"Record: {record}")
    
    assert record["created_at"] is None, "created_at should be None"
    assert record["min_lat"] is None, "bbox should be None"
    assert record["replication_sequence"] is None
    
    # Write to Parquet
    print("Writing to Parquet...")
    writer = PartitionedWriter(output_dir)
    writer.write_changesets(handler.buffer)
    writer.finish()
    
    # Verify fallback partition
    fallback_file = output_dir / "year=1970/month=01/data.parquet"
    if fallback_file.exists():
        print(f"SUCCESS: Written to fallback partition {fallback_file}")
    else:
        print(f"FAILURE: Fallback partition not found in {list(output_dir.glob('**/*'))}")

    # Cleanup
    input_file.unlink()
    shutil.rmtree(output_dir)

if __name__ == "__main__":
    test_legacy_handling()
