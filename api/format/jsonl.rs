//! JSONL (JSON Lines) format output
//!
//! Formats changesets in JSONL format - one JSON object per line.
//! Best for streaming and bulk processing of large datasets.

use crate::models::Changeset;
use crate::format::Formatter;
use serde::Serialize;
use std::collections::HashMap;
use chrono::{TimeZone, Utc, SecondsFormat};

/// JSONL formatter for changeset output
pub struct JsonLinesFormatter;

#[derive(Debug, Serialize)]
struct JsonLinesChangeset {
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    closed_at: Option<String>,
    open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uid: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_lon: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_lon: Option<f64>,
    #[serde(rename = "changes_count")]
    num_changes: u32,
    comments_count: u32,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    tags: HashMap<String, String>,
}

impl JsonLinesFormatter {
    /// Create a new JSONL formatter
    pub fn new() -> Self {
        Self
    }

    /// Convert Changeset to JsonLinesChangeset for serialization
    fn to_jsonl_changeset(&self, changeset: &Changeset) -> JsonLinesChangeset {
        let mut tags = changeset.tags.clone();
        if let Some(description) = &changeset.description {
            tags.entry("comment".to_string()).or_insert_with(|| description.clone());
        }

        JsonLinesChangeset {
            id: changeset.id,
            created_at: changeset.created_at.and_then(|ts| {
                Utc.timestamp_millis_opt(ts).single().map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            }),
            closed_at: changeset.closed_at.and_then(|ts| {
                Utc.timestamp_millis_opt(ts).single().map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            }),
            open: changeset.open,
            user: changeset.user.clone(),
            uid: changeset.uid,
            min_lat: changeset.min_lat,
            min_lon: changeset.min_lon,
            max_lat: changeset.max_lat,
            max_lon: changeset.max_lon,
            num_changes: changeset.num_changes,
            comments_count: changeset.comments_count,
            tags,
        }
    }
}

impl Formatter for JsonLinesFormatter {
    fn format_changeset(&self, changeset: &Changeset) -> Result<String, Box<dyn std::error::Error>> {
        let jsonl_changeset = self.to_jsonl_changeset(changeset);
        Ok(serde_json::to_string(&jsonl_changeset)?)
    }

    fn format_changesets(&self, changesets: &[Changeset]) -> Result<String, Box<dyn std::error::Error>> {
        let lines: Vec<String> = changesets
            .iter()
            .map(|cs| {
                let jsonl_changeset = self.to_jsonl_changeset(cs);
                serde_json::to_string(&jsonl_changeset)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Changeset;
    use serde_json::Value;
    use std::collections::HashMap;

    fn build_changeset(id: i64, description: Option<&str>, tags: HashMap<String, String>) -> Changeset {
        Changeset {
            id,
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
    fn test_single_changeset() {
        let changeset = build_changeset(123, Some("test"), HashMap::new());
        let formatter = JsonLinesFormatter::new();
        let jsonl = formatter.format_changeset(&changeset).expect("jsonl");

        let value: Value = serde_json::from_str(&jsonl).expect("parse jsonl");
        assert_eq!(value["id"], 123);
        assert_eq!(value["tags"]["comment"], "test");
    }

    #[test]
    fn test_multiple_changesets() {
        let changesets = vec![
            build_changeset(1, Some("first"), HashMap::new()),
            build_changeset(2, Some("second"), HashMap::new()),
        ];

        let formatter = JsonLinesFormatter::new();
        let jsonl = formatter.format_changesets(&changesets).expect("jsonl");

        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).expect("parse first");
        let second: Value = serde_json::from_str(lines[1]).expect("parse second");

        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
    }

    #[test]
    fn test_empty_tags_omitted() {
        let changeset = build_changeset(456, None, HashMap::new());
        let formatter = JsonLinesFormatter::new();
        let jsonl = formatter.format_changeset(&changeset).expect("jsonl");

        let value: Value = serde_json::from_str(&jsonl).expect("parse jsonl");
        assert!(value.get("tags").is_none());
    }
}
