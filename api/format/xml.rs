//! OSM XML format output
//!
//! Formats changesets in OSM XML format, compatible with the official OSM replication format.

use crate::format::{Formatter, StreamingFormatter};
use crate::generator_string;
use crate::models::Changeset;
use chrono::{SecondsFormat, TimeZone, Utc};
use std::borrow::Cow;

fn osm_root_open() -> String {
    format!(
        "<osm version=\"0.6\" generator=\"{}\" copyright=\"OpenStreetMap and contributors\" attribution=\"http://www.openstreetmap.org/copyright\" license=\"http://opendatacommons.org/licenses/odbl/1-0/\">\n",
        generator_string()
    )
}

/// XML formatter for OSM changeset output
pub struct XmlFormatter;

impl XmlFormatter {
    /// Create a new XML formatter
    pub fn new() -> Self {
        Self
    }

    /// Escape XML special characters
    fn escape_xml(s: &str) -> Cow<'_, str> {
        if s.contains(['&', '<', '>', '"', '\'']) {
            let mut result = String::with_capacity(s.len() + 10);
            for c in s.chars() {
                match c {
                    '&' => result.push_str("&amp;"),
                    '<' => result.push_str("&lt;"),
                    '>' => result.push_str("&gt;"),
                    '"' => result.push_str("&quot;"),
                    '\'' => result.push_str("&apos;"),
                    _ => result.push(c),
                }
            }
            Cow::Owned(result)
        } else {
            Cow::Borrowed(s)
        }
    }

    /// Format a single changeset as XML element string
    fn format_changeset_element(&self, changeset: &Changeset) -> String {
        let mut xml = String::with_capacity(512);

        // Start changeset tag
        xml.push_str(&format!("\t<changeset id=\"{}\"", changeset.id));

        // Add timestamps
        if let Some(created_at) = changeset.created_at {
            if let Some(dt) = Utc.timestamp_millis_opt(created_at).single() {
                xml.push_str(&format!(
                    " created_at=\"{}\"",
                    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
                ));
            }
        }

        if let Some(closed_at) = changeset.closed_at {
            if let Some(dt) = Utc.timestamp_millis_opt(closed_at).single() {
                xml.push_str(&format!(
                    " closed_at=\"{}\"",
                    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
                ));
            }
        }

        xml.push_str(&format!(" open=\"{}\"", changeset.open));

        if let Some(user) = &changeset.user {
            xml.push_str(&format!(" user=\"{}\"", Self::escape_xml(user)));
        }

        if let Some(uid) = changeset.uid {
            xml.push_str(&format!(" uid=\"{}\"", uid));
        }

        if let Some(min_lat) = changeset.min_lat {
            xml.push_str(&format!(" min_lat=\"{}\"", min_lat));
        }
        if let Some(min_lon) = changeset.min_lon {
            xml.push_str(&format!(" min_lon=\"{}\"", min_lon));
        }
        if let Some(max_lat) = changeset.max_lat {
            xml.push_str(&format!(" max_lat=\"{}\"", max_lat));
        }
        if let Some(max_lon) = changeset.max_lon {
            xml.push_str(&format!(" max_lon=\"{}\"", max_lon));
        }

        xml.push_str(&format!(" comments_count=\"{}\"", changeset.comments_count));
        xml.push_str(&format!(" changes_count=\"{}\"", changeset.num_changes));

        let tags = self.collect_tags(changeset);
        if tags.is_empty() {
            xml.push_str("/>\n");
        } else {
            xml.push_str(">\n");
            xml.push_str(&self.format_tags(&tags));
            xml.push_str("\t</changeset>\n");
        }

        xml
    }

    /// Format tags as XML elements
    fn format_tags(&self, tags: &[(String, String)]) -> String {
        let mut tags_xml = String::new();
        for (k, v) in tags {
            tags_xml.push_str(&format!(
                "\t\t<tag k=\"{}\" v=\"{}\"/>\n",
                Self::escape_xml(k),
                Self::escape_xml(v)
            ));
        }
        tags_xml
    }

    fn collect_tags(&self, changeset: &Changeset) -> Vec<(String, String)> {
        let mut tags: Vec<(String, String)> = changeset
            .tags
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if let Some(description) = &changeset.description {
            if !tags.iter().any(|(k, _)| k == "comment") {
                tags.push(("comment".to_string(), description.clone()));
            }
        }

        tags.sort_by(|(a, _), (b, _)| a.cmp(b));
        tags
    }
}

impl Formatter for XmlFormatter {
    fn format_changeset(
        &self,
        changeset: &Changeset,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(&osm_root_open());
        xml.push_str(&self.format_changeset_element(changeset));
        xml.push_str("</osm>");
        Ok(xml)
    }

    fn format_changesets(
        &self,
        changesets: &[Changeset],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(&osm_root_open());
        for changeset in changesets {
            xml.push_str(&self.format_changeset_element(changeset));
        }
        xml.push_str("</osm>");
        Ok(xml)
    }
}

impl StreamingFormatter for XmlFormatter {
    fn content_type(&self) -> &'static str {
        "application/xml"
    }

    fn stream_header(&self) -> String {
        let mut header = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        header.push_str(&osm_root_open());
        header
    }

    fn stream_item(
        &self,
        changeset: &Changeset,
        _is_first: bool,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.format_changeset_element(changeset))
    }

    fn stream_footer(&self) -> String {
        "</osm>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Changeset;
    use std::collections::HashMap;

    fn build_changeset(
        id: i64,
        description: Option<&str>,
        tags: HashMap<String, String>,
    ) -> Changeset {
        Changeset {
            id,
            replication_sequence: None,
            created_at: None,
            closed_at: None,
            open: false,
            user: None,
            uid: None,
            min_lat: None,
            min_lon: None,
            max_lat: None,
            max_lon: None,
            num_changes: 0,
            comments_count: 0,
            description: description.map(|value| value.to_string()),
            tags,
        }
    }

    #[test]
    fn test_comment_tag_from_description() {
        let changeset = build_changeset(1, Some("hello"), HashMap::new());
        let formatter = XmlFormatter::new();
        let xml = formatter.format_changeset(&changeset).expect("xml");

        assert!(xml.contains("k=\"comment\" v=\"hello\""));
    }

    #[test]
    fn test_existing_comment_not_duplicated() {
        let mut tags = HashMap::new();
        tags.insert("comment".to_string(), "custom".to_string());

        let changeset = build_changeset(2, Some("ignored"), tags);
        let formatter = XmlFormatter::new();
        let xml = formatter.format_changeset(&changeset).expect("xml");

        assert!(xml.contains("k=\"comment\" v=\"custom\""));
        assert_eq!(xml.matches("k=\"comment\"").count(), 1);
    }

    #[test]
    fn test_xml_escapes_special_chars() {
        let changeset = build_changeset(3, Some("A & B < C"), HashMap::new());
        let formatter = XmlFormatter::new();
        let xml = formatter.format_changeset(&changeset).expect("xml");

        assert!(xml.contains("v=\"A &amp; B &lt; C\""));
    }

    #[test]
    fn test_root_attributes_present() {
        let changeset = build_changeset(4, None, HashMap::new());
        let formatter = XmlFormatter::new();
        let xml = formatter.format_changeset(&changeset).expect("xml");

        assert!(xml.contains("copyright=\"OpenStreetMap and contributors\""));
        assert!(xml.contains("attribution=\"http://www.openstreetmap.org/copyright\""));
        assert!(xml.contains("license=\"http://opendatacommons.org/licenses/odbl/1-0/\""));
    }
}
