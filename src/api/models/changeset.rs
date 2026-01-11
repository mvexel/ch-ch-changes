use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Represents an OSM changeset with all its metadata.
/// Matches the Parquet schema from the changeset dump.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    /// Unique changeset identifier
    pub id: i64,

    /// Timestamp when the changeset was created (milliseconds since epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,

    /// Timestamp when the changeset was closed (milliseconds since epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<i64>,

    /// Whether the changeset is still open
    pub open: bool,

    /// Username of the mapper who created the changeset
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// User ID of the mapper
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<i64>,

    /// Minimum latitude of the changeset bounding box
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_lat: Option<f64>,

    /// Minimum longitude of the changeset bounding box
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_lon: Option<f64>,

    /// Maximum latitude of the changeset bounding box
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lat: Option<f64>,

    /// Maximum longitude of the changeset bounding box
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lon: Option<f64>,

    /// Number of changes (nodes, ways, relations) in the changeset
    pub num_changes: u32,

    /// Number of discussion comments on the changeset
    pub comments_count: u32,

    /// Change description/comment provided by the mapper
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Changeset {
    /// Convert a timestamp in milliseconds since epoch to an RFC3339 formatted string
    pub fn format_timestamp(timestamp_ms: Option<i64>) -> Option<String> {
        timestamp_ms.and_then(|ms| {
            // Convert milliseconds to seconds and nanoseconds
            let secs = ms / 1000;
            let nanos = ((ms % 1000) * 1_000_000) as u32;

            // Create DateTime from timestamp
            match DateTime::<Utc>::from_timestamp(secs, nanos) {
                Some(dt) => Some(dt.to_rfc3339()),
                None => None,
            }
        })
    }

    /// Convert created_at timestamp to RFC3339 string
    pub fn created_at_rfc3339(&self) -> Option<String> {
        Self::format_timestamp(self.created_at)
    }

    /// Convert closed_at timestamp to RFC3339 string
    pub fn closed_at_rfc3339(&self) -> Option<String> {
        Self::format_timestamp(self.closed_at)
    }

    /// Check if the changeset has a valid bounding box
    pub fn has_bbox(&self) -> bool {
        self.min_lat.is_some()
            && self.min_lon.is_some()
            && self.max_lat.is_some()
            && self.max_lon.is_some()
    }

    /// Get the bounding box as a tuple if available
    pub fn bbox(&self) -> Option<(f64, f64, f64, f64)> {
        if self.has_bbox() {
            Some((
                self.min_lat.unwrap(),
                self.min_lon.unwrap(),
                self.max_lat.unwrap(),
                self.max_lon.unwrap(),
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        // Test with a known timestamp: 2024-10-16T08:00:00Z = 1729070400 seconds = 1729070400000 milliseconds
        let timestamp_ms = Some(1729070400000i64);
        let formatted = Changeset::format_timestamp(timestamp_ms);
        assert!(formatted.is_some());
        let formatted_str = formatted.unwrap();
        // RFC3339 format includes timezone, check for the date and time portion
        assert!(formatted_str.contains("2024-10-16"),
                "Expected timestamp to contain date, got: {}", formatted_str);
        // Also verify it's a valid RFC3339 string
        assert!(formatted_str.contains("T") && (formatted_str.contains("Z") || formatted_str.contains("+")),
                "Expected RFC3339 format, got: {}", formatted_str);
    }

    #[test]
    fn test_format_timestamp_none() {
        let formatted = Changeset::format_timestamp(None);
        assert!(formatted.is_none());
    }

    #[test]
    fn test_bbox_check() {
        let changeset_with_bbox = Changeset {
            id: 1,
            created_at: None,
            closed_at: None,
            open: false,
            user: None,
            uid: None,
            min_lat: Some(40.0),
            min_lon: Some(-120.0),
            max_lat: Some(41.0),
            max_lon: Some(-119.0),
            num_changes: 10,
            comments_count: 0,
            description: None,
        };

        assert!(changeset_with_bbox.has_bbox());
        let bbox = changeset_with_bbox.bbox().unwrap();
        assert_eq!(bbox, (40.0, -120.0, 41.0, -119.0));
    }

    #[test]
    fn test_bbox_missing() {
        let changeset_no_bbox = Changeset {
            id: 1,
            created_at: None,
            closed_at: None,
            open: false,
            user: None,
            uid: None,
            min_lat: None,
            min_lon: None,
            max_lat: None,
            max_lon: None,
            num_changes: 10,
            comments_count: 0,
            description: None,
        };

        assert!(!changeset_no_bbox.has_bbox());
        assert!(changeset_no_bbox.bbox().is_none());
    }
}
