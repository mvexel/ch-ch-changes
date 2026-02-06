use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

pub mod builder;
#[cfg(feature = "indexing")]
pub mod indexing;
pub mod models;
pub mod parsing;
pub mod schema;
pub mod writer;

/// Represents an OpenStreetMap changeset
///
/// This structure matches the schema of the Parquet files and is used
/// for both the converter tool and API server.
///
/// # Fields
/// - `id` - Unique changeset identifier
/// - `created_at` - Changeset creation timestamp (milliseconds since epoch)
/// - `closed_at` - Changeset closure timestamp (milliseconds since epoch), None if still open
/// - `open` - Whether the changeset is still open
/// - `user` - Username of changeset author
/// - `uid` - User ID of changeset author
/// - `min_lat` - Minimum latitude of changeset bounding box
/// - `min_lon` - Minimum longitude of changeset bounding box
/// - `max_lat` - Maximum latitude of changeset bounding box
/// - `max_lon` - Maximum longitude of changeset bounding box
/// - `num_changes` - Number of objects (nodes, ways, relations) changed
/// - `comments_count` - Number of discussion comments on this changeset
/// - `description` - Changeset description/comment
/// - `tags` - Additional key-value pairs (e.g., "comment", "review_requested")
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Changeset {
    pub id: i64,
    pub replication_sequence: Option<u64>,
    pub created_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub open: bool,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub min_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lat: Option<f64>,
    pub max_lon: Option<f64>,
    pub num_changes: u32,
    pub comments_count: u32,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub tags: HashMap<String, String>,
}

#[allow(dead_code)]
impl Changeset {
    /// Create a new changeset with minimal required fields
    pub fn new(id: i64) -> Self {
        Self {
            id,
            replication_sequence: None,
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
            tags: HashMap::new(),
        }
    }

    /// Check if this changeset has a valid bounding box
    pub fn has_valid_bbox(&self) -> bool {
        match (self.min_lat, self.min_lon, self.max_lat, self.max_lon) {
            (Some(min_lat), Some(min_lon), Some(max_lat), Some(max_lon)) => {
                min_lat >= -90.0
                    && max_lat <= 90.0
                    && min_lat <= max_lat
                    && min_lon >= -180.0
                    && max_lon <= 180.0
            }
            _ => false,
        }
    }

    /// Get the changeset's duration in seconds (if closed)
    pub fn duration_seconds(&self) -> Option<i64> {
        match (self.created_at, self.closed_at) {
            (Some(created), Some(closed)) if closed >= created => Some((closed - created) / 1000),
            _ => None,
        }
    }
}
