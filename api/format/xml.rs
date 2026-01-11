//! OSM XML format output
//!
//! Formats changesets in OSM XML format, compatible with the official OSM replication format.

use crate::models::Changeset;
use crate::format::Formatter;
use chrono::{TimeZone, Utc, SecondsFormat};
use std::borrow::Cow;

/// XML formatter for OSM changeset output
pub struct XmlFormatter;

impl XmlFormatter {
    /// Create a new XML formatter
    pub fn new() -> Self {
        Self
    }

    /// Escape XML special characters
    fn escape_xml(s: &str) -> Cow<str> {
        if s.contains(|c| matches!(c, '&' | '<' | '>' | '"' | '\'')) {
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
                xml.push_str(&format!(" created_at=\"{}\"", 
                    dt.to_rfc3339_opts(SecondsFormat::Secs, true)));
            }
        }
        
        if let Some(closed_at) = changeset.closed_at {
            if let Some(dt) = Utc.timestamp_millis_opt(closed_at).single() {
                xml.push_str(&format!(" closed_at=\"{}\"", 
                    dt.to_rfc3339_opts(SecondsFormat::Secs, true)));
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

        xml.push_str(&format!(" num_changes=\"{}\"", changeset.num_changes));
        xml.push_str(&format!(" comments_count=\"{}\"", changeset.comments_count));

        if changeset.tags.is_empty() {
            xml.push_str("/>\n");
        } else {
            xml.push_str(">\n");
            xml.push_str(&self.format_tags(changeset));
            xml.push_str("\t</changeset>\n");
        }

        xml
    }

    /// Format tags as XML elements
    fn format_tags(&self, changeset: &Changeset) -> String {
        let mut tags_xml = String::new();
        // Sort tags by key for consistent output (optional but good for testing)
        let mut sorted_tags: Vec<_> = changeset.tags.iter().collect();
        sorted_tags.sort_by_key(|(k, _)| *k);

        for (k, v) in sorted_tags {
            tags_xml.push_str(&format!("\t\t<tag k=\"{}\" v=\"{}\"/>\n", 
                Self::escape_xml(k), 
                Self::escape_xml(v)));
        }
        tags_xml
    }
}

impl Formatter for XmlFormatter {
    fn format_changeset(&self, changeset: &Changeset) -> Result<String, Box<dyn std::error::Error>> {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<osm version=\"0.6\" generator=\"changeset-historic-api/0.1.0\">\n");
        xml.push_str(&self.format_changeset_element(changeset));
        xml.push_str("</osm>");
        Ok(xml)
    }

    fn format_changesets(&self, changesets: &[Changeset]) -> Result<String, Box<dyn std::error::Error>> {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<osm version=\"0.6\" generator=\"changeset-historic-api/0.1.0\">\n");
        for changeset in changesets {
            xml.push_str(&self.format_changeset_element(changeset));
        }
        xml.push_str("</osm>");
        Ok(xml)
    }
}
