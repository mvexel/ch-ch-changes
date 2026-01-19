//! JSON format output
//!
//! Formats changesets in JSON format.

use crate::format::{Formatter, StreamingFormatter};
use crate::generator_string;
use crate::models::Changeset;
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON formatter for changeset output
pub struct JsonFormatter {
    pretty: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonResponse {
    version: String,
    generator: String,
    copyright: String,
    attribution: String,
    license: String,
    changesets: Vec<JsonChangeset>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SingleJsonResponse {
    version: String,
    generator: String,
    copyright: String,
    attribution: String,
    license: String,
    changeset: JsonChangeset,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonChangeset {
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

impl JsonFormatter {
    /// Create a new JSON formatter
    pub fn new(pretty: bool) -> Self {
        Self { pretty }
    }

    /// Convert Changeset to JsonChangeset for serialization
    fn to_json_changeset(&self, changeset: &Changeset) -> JsonChangeset {
        let mut tags = changeset.tags.clone();
        if let Some(description) = &changeset.description {
            tags.entry("comment".to_string())
                .or_insert_with(|| description.clone());
        }

        JsonChangeset {
            id: changeset.id,
            created_at: changeset.created_at.and_then(|ts| {
                Utc.timestamp_millis_opt(ts)
                    .single()
                    .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            }),
            closed_at: changeset.closed_at.and_then(|ts| {
                Utc.timestamp_millis_opt(ts)
                    .single()
                    .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
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

impl Formatter for JsonFormatter {
    fn format_changeset(
        &self,
        changeset: &Changeset,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let response = SingleJsonResponse {
            version: "0.6".to_string(),
            generator: generator_string(),
            copyright: "OpenStreetMap and contributors".to_string(),
            attribution: "http://www.openstreetmap.org/copyright".to_string(),
            license: "http://opendatacommons.org/licenses/odbl/1-0/".to_string(),
            changeset: self.to_json_changeset(changeset),
        };

        if self.pretty {
            Ok(serde_json::to_string_pretty(&response)?)
        } else {
            Ok(serde_json::to_string(&response)?)
        }
    }

    fn format_changesets(
        &self,
        changesets: &[Changeset],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let response = JsonResponse {
            version: "0.6".to_string(),
            generator: generator_string(),
            copyright: "OpenStreetMap and contributors".to_string(),
            attribution: "http://www.openstreetmap.org/copyright".to_string(),
            license: "http://opendatacommons.org/licenses/odbl/1-0/".to_string(),
            changesets: changesets
                .iter()
                .map(|c| self.to_json_changeset(c))
                .collect(),
        };

        if self.pretty {
            Ok(serde_json::to_string_pretty(&response)?)
        } else {
            Ok(serde_json::to_string(&response)?)
        }
    }
}

impl StreamingFormatter for JsonFormatter {
    fn content_type(&self) -> &'static str {
        "application/json"
    }

    fn stream_header(&self) -> String {
        format!(
            r#"{{"version":"0.6","generator":"{}","copyright":"OpenStreetMap and contributors","attribution":"http://www.openstreetmap.org/copyright","license":"http://opendatacommons.org/licenses/odbl/1-0/","changesets":["#,
            generator_string()
        )
    }

    fn stream_item(&self, changeset: &Changeset, is_first: bool) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let json_changeset = self.to_json_changeset(changeset);
        let item = serde_json::to_string(&json_changeset)?;
        if is_first {
            Ok(item)
        } else {
            Ok(format!(",{}", item))
        }
    }

    fn stream_footer(&self) -> String {
        "]}".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Changeset;
    use serde_json::Value;
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
        let formatter = JsonFormatter::new(false);
        let json = formatter.format_changeset(&changeset).expect("json");
        let value: Value = serde_json::from_str(&json).expect("parse json");

        assert_eq!(value["changeset"]["id"], 1);
        assert_eq!(value["changeset"]["tags"]["comment"], "hello");
    }

    #[test]
    fn test_existing_comment_not_overwritten() {
        let mut tags = HashMap::new();
        tags.insert("comment".to_string(), "custom".to_string());

        let changeset = build_changeset(2, Some("ignored"), tags);
        let formatter = JsonFormatter::new(false);
        let json = formatter.format_changeset(&changeset).expect("json");
        let value: Value = serde_json::from_str(&json).expect("parse json");

        assert_eq!(value["changeset"]["tags"]["comment"], "custom");
    }

    #[test]
    fn test_empty_tags_omitted() {
        let changeset = build_changeset(3, None, HashMap::new());
        let formatter = JsonFormatter::new(false);
        let json = formatter.format_changeset(&changeset).expect("json");
        let value: Value = serde_json::from_str(&json).expect("parse json");

        assert!(value["changeset"].get("tags").is_none());
    }
}
