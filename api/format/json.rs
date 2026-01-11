//! JSON format output
//!
//! Formats changesets in JSON format.

use crate::models::Changeset;
use crate::format::Formatter;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use chrono::{TimeZone, Utc, SecondsFormat};

/// JSON formatter for changeset output
pub struct JsonFormatter {
    pretty: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonResponse {
    version: String,
    generator: String,
    changesets: Vec<JsonChangeset>,
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
        JsonChangeset {
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
            tags: changeset.tags.clone(),
        }
    }
}

impl Formatter for JsonFormatter {
    fn format_changeset(&self, changeset: &Changeset) -> Result<String, Box<dyn std::error::Error>> {
        let response = JsonResponse {
            version: "0.6".to_string(),
            generator: "changeset-historic-api/0.1.0".to_string(),
            changesets: vec![self.to_json_changeset(changeset)],
        };

        if self.pretty {
            Ok(serde_json::to_string_pretty(&response)?)
        } else {
            Ok(serde_json::to_string(&response)?)
        }
    }

    fn format_changesets(&self, changesets: &[Changeset]) -> Result<String, Box<dyn std::error::Error>> {
        let response = JsonResponse {
            version: "0.6".to_string(),
            generator: "changeset-historic-api/0.1.0".to_string(),
            changesets: changesets.iter().map(|c| self.to_json_changeset(c)).collect(),
        };

        if self.pretty {
            Ok(serde_json::to_string_pretty(&response)?)
        } else {
            Ok(serde_json::to_string(&response)?)
        }
    }
}
