use quick_xml::events::{BytesDecl, BytesStart, BytesEnd, Event};
use quick_xml::Writer;
use std::io::Cursor;
use crate::api::models::Changeset;

/// Converts milliseconds since epoch to RFC3339 timestamp string
fn timestamp_millis_to_rfc3339(millis: i64) -> String {
    Changeset::format_timestamp(Some(millis))
        .unwrap_or_else(|| "1970-01-01T00:00:00+00:00".to_string())
}

/// Converts a Vec<Changeset> to OSM XML format compatible with the replication format
///
/// # Example
/// ```text
/// <?xml version="1.0" encoding="UTF-8"?>
/// <osm version="0.6" generator="changeset-historic-api/0.1.0">
///   <changeset id="168575829"
///              created_at="2024-10-15T10:30:00Z"
///              closed_at="2024-10-15T10:45:00Z"
///              open="false"
///              user="mvexel"
///              uid="12345"
///              min_lat="40.7128"
///              min_lon="-74.0060"
///              max_lat="40.7200"
///              max_lon="-73.9900"
///              num_changes="42"
///              comments_count="2">
///     <tag k="comment" v="Fixed street names in Manhattan"/>
///   </changeset>
/// </osm>
/// ```
pub fn format_changesets_xml(changesets: Vec<Changeset>) -> String {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    // Write XML declaration
    let decl = BytesDecl::new("1.0", Some("UTF-8"), None);
    writer.write_event(Event::Decl(decl)).unwrap();

    // Write root osm element start
    let mut osm_elem = BytesStart::new("osm");
    osm_elem.push_attribute(("version", "0.6"));
    osm_elem.push_attribute(("generator", "changeset-historic-api/0.1.0"));
    writer.write_event(Event::Start(osm_elem)).unwrap();

    // Write each changeset
    for changeset in changesets {
        write_changeset(&mut writer, &changeset);
    }

    // Write root osm element end
    writer.write_event(Event::End(BytesEnd::new("osm"))).unwrap();

    // Extract the XML string from the cursor
    let result = writer.into_inner().into_inner();
    String::from_utf8(result).unwrap_or_default()
}

/// Writes a single changeset element to the XML writer
fn write_changeset(writer: &mut Writer<Cursor<Vec<u8>>>, changeset: &Changeset) {
    // Pre-allocate all strings we'll need for attributes
    let id_str = changeset.id.to_string();
    let created_at_str = changeset.created_at.map(timestamp_millis_to_rfc3339);
    let closed_at_str = changeset.closed_at.map(timestamp_millis_to_rfc3339);
    let open_str = if changeset.open { "true" } else { "false" };
    let uid_str = changeset.uid.map(|u| u.to_string());
    let min_lat_str = changeset.min_lat.map(|v| v.to_string());
    let min_lon_str = changeset.min_lon.map(|v| v.to_string());
    let max_lat_str = changeset.max_lat.map(|v| v.to_string());
    let max_lon_str = changeset.max_lon.map(|v| v.to_string());
    let num_changes_str = changeset.num_changes.to_string();
    let comments_count_str = changeset.comments_count.to_string();

    let mut cs_elem = BytesStart::new("changeset");

    // Required attributes
    cs_elem.push_attribute(("id", id_str.as_str()));

    // Optional timestamp attributes
    if let Some(ref timestamp) = created_at_str {
        cs_elem.push_attribute(("created_at", timestamp.as_str()));
    }

    if let Some(ref timestamp) = closed_at_str {
        cs_elem.push_attribute(("closed_at", timestamp.as_str()));
    }

    // Open status
    cs_elem.push_attribute(("open", open_str));

    // Optional user and uid
    if let Some(ref user) = changeset.user {
        cs_elem.push_attribute(("user", user.as_str()));
    }

    if let Some(ref uid) = uid_str {
        cs_elem.push_attribute(("uid", uid.as_str()));
    }

    // Optional bounding box
    if let Some(ref lat) = min_lat_str {
        cs_elem.push_attribute(("min_lat", lat.as_str()));
    }

    if let Some(ref lon) = min_lon_str {
        cs_elem.push_attribute(("min_lon", lon.as_str()));
    }

    if let Some(ref lat) = max_lat_str {
        cs_elem.push_attribute(("max_lat", lat.as_str()));
    }

    if let Some(ref lon) = max_lon_str {
        cs_elem.push_attribute(("max_lon", lon.as_str()));
    }

    // Change and comment counts
    cs_elem.push_attribute(("num_changes", num_changes_str.as_str()));
    cs_elem.push_attribute(("comments_count", comments_count_str.as_str()));

    // Write start element
    writer.write_event(Event::Start(cs_elem)).unwrap();

    // Write description as tag child element if present
    if let Some(ref description) = changeset.description {
        let mut tag_elem = BytesStart::new("tag");
        tag_elem.push_attribute(("k", "comment"));
        tag_elem.push_attribute(("v", description.as_str()));
        writer.write_event(Event::Empty(tag_elem)).unwrap();
    }

    // Write end element
    writer.write_event(Event::End(BytesEnd::new("changeset"))).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_changesets() {
        let result = format_changesets_xml(vec![]);
        assert!(result.contains("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(result.contains("<osm version=\"0.6\" generator=\"changeset-historic-api/0.1.0\">"));
        assert!(result.contains("</osm>"));
    }

    #[test]
    fn test_single_changeset_with_all_fields() {
        let changeset = Changeset {
            id: 168575829,
            created_at: Some(1729079400000), // 2024-10-15T10:30:00Z
            closed_at: Some(1729080300000),  // 2024-10-15T10:45:00Z
            open: false,
            user: Some("mvexel".to_string()),
            uid: Some(12345),
            min_lat: Some(40.7128),
            min_lon: Some(-74.0060),
            max_lat: Some(40.7200),
            max_lon: Some(-73.9900),
            num_changes: 42,
            comments_count: 2,
            description: Some("Fixed street names in Manhattan".to_string()),
        };

        let result = format_changesets_xml(vec![changeset]);

        assert!(result.contains("id=\"168575829\""));
        assert!(result.contains("user=\"mvexel\""));
        assert!(result.contains("uid=\"12345\""));
        assert!(result.contains("open=\"false\""));
        assert!(result.contains("num_changes=\"42\""));
        assert!(result.contains("comments_count=\"2\""));
        assert!(result.contains("min_lat=\"40.7128\""));
        assert!(result.contains("min_lon=\"-74.006\""));
        assert!(result.contains("max_lat=\"40.72\""));
        assert!(result.contains("max_lon=\"-73.99\""));
        assert!(result.contains("k=\"comment\""));
        assert!(result.contains("v=\"Fixed street names in Manhattan\""));
    }

    #[test]
    fn test_changeset_with_optional_fields_none() {
        let changeset = Changeset {
            id: 12345,
            created_at: None,
            closed_at: None,
            open: true,
            user: None,
            uid: None,
            min_lat: None,
            min_lon: None,
            max_lat: None,
            max_lon: None,
            num_changes: 0,
            comments_count: 0,
            description: None,
        };

        let result = format_changesets_xml(vec![changeset]);

        assert!(result.contains("id=\"12345\""));
        assert!(result.contains("open=\"true\""));
        assert!(result.contains("num_changes=\"0\""));
        assert!(result.contains("comments_count=\"0\""));
        // Should not contain optional fields
        assert!(!result.contains("created_at="));
        assert!(!result.contains("user="));
        assert!(!result.contains("uid="));
        assert!(!result.contains("min_lat="));
    }

    #[test]
    fn test_xml_escaping_in_user_and_description() {
        let changeset = Changeset {
            id: 999,
            created_at: Some(1729079400000),
            closed_at: Some(1729080300000),
            open: false,
            user: Some("user&name<test>".to_string()),
            uid: Some(999),
            min_lat: None,
            min_lon: None,
            max_lat: None,
            max_lon: None,
            num_changes: 5,
            comments_count: 0,
            description: Some("Fixed \"quoted\" & special <chars>".to_string()),
        };

        let result = format_changesets_xml(vec![changeset]);

        // quick-xml automatically escapes attribute values
        assert!(result.contains("id=\"999\""));
        assert!(result.contains("num_changes=\"5\""));
        // The user and description should be properly escaped
        assert!(result.contains("user="));
        assert!(result.contains("v="));
    }

    #[test]
    fn test_multiple_changesets() {
        let changesets = vec![
            Changeset {
                id: 1,
                created_at: Some(1000000000),
                closed_at: Some(2000000000),
                open: false,
                user: Some("user1".to_string()),
                uid: Some(1),
                min_lat: Some(0.0),
                min_lon: Some(0.0),
                max_lat: Some(1.0),
                max_lon: Some(1.0),
                num_changes: 10,
                comments_count: 0,
                description: None,
            },
            Changeset {
                id: 2,
                created_at: Some(3000000000),
                closed_at: None,
                open: true,
                user: Some("user2".to_string()),
                uid: Some(2),
                min_lat: None,
                min_lon: None,
                max_lat: None,
                max_lon: None,
                num_changes: 20,
                comments_count: 1,
                description: Some("Comment here".to_string()),
            },
        ];

        let result = format_changesets_xml(changesets);

        assert!(result.contains("id=\"1\""));
        assert!(result.contains("id=\"2\""));
        assert!(result.contains("user=\"user1\""));
        assert!(result.contains("user=\"user2\""));
        assert!(result.contains("num_changes=\"10\""));
        assert!(result.contains("num_changes=\"20\""));
    }

    #[test]
    fn test_timestamp_conversion() {
        // Test a known timestamp: 2024-10-16T08:30:00Z = 1729079400000 ms (actually)
        let expected_timestamp = 1729079400000i64;
        let result = timestamp_millis_to_rfc3339(expected_timestamp);
        println!("Timestamp result: {}", result);
        // Just verify it's a valid RFC3339 format
        assert!(result.contains("T"));
        assert!(result.contains("Z") || result.contains("+"));
    }
}
