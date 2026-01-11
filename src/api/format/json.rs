use crate::api::models::Changeset;
use chrono::{DateTime, Utc};
use serde::{Serialize, Serializer};

/// Helper struct for serializing timestamps as RFC3339 strings
fn serialize_timestamp<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(ts_millis) => {
            let dt = DateTime::<Utc>::from_timestamp_millis(*ts_millis)
                .ok_or_else(|| serde::ser::Error::custom("Invalid timestamp"))?;
            let rfc3339 = dt.to_rfc3339();
            serializer.serialize_some(&rfc3339)
        }
        None => serializer.serialize_none(),
    }
}

/// JSON representation of a single changeset
#[derive(Serialize)]
struct ChangesetJson {
    id: i64,
    #[serde(serialize_with = "serialize_timestamp")]
    created_at: Option<i64>,
    #[serde(serialize_with = "serialize_timestamp")]
    closed_at: Option<i64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Tags>,
}

/// Tags object containing comment (from description field)
#[derive(Serialize)]
struct Tags {
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
}

/// Root JSON structure for changeset output
#[derive(Serialize)]
struct ChangesetOutput {
    version: String,
    generator: String,
    changesets: Vec<ChangesetJson>,
}


/// Convert a Vec<Changeset> to JSON format
///
/// Produces JSON output matching the structure:
/// ```json
/// {
///   "version": "0.6",
///   "generator": "changeset-historic-api/0.1.0",
///   "changesets": [
///     {
///       "id": 168575829,
///       "created_at": "2024-10-15T10:30:00Z",
///       "closed_at": "2024-10-15T10:45:00Z",
///       "open": false,
///       "user": "mvexel",
///       "uid": 12345,
///       "min_lat": 40.7128,
///       "min_lon": -74.0060,
///       "max_lat": 40.7200,
///       "max_lon": -73.9900,
///       "num_changes": 42,
///       "comments_count": 2,
///       "tags": {
///         "comment": "Fixed street names in Manhattan"
///       }
///     }
///   ]
/// }
/// ```
///
/// # Arguments
///
/// * `changesets` - A vector of Changeset structs to convert to JSON
///
/// # Returns
///
/// * `Result<String, serde_json::Error>` - JSON string or error
///
/// # Features
///
/// - Timestamps are converted from milliseconds to RFC3339 strings
/// - Description field becomes tags.comment
/// - Null fields are omitted from output (skip_serializing_if)
/// - Version is hardcoded to "0.6"
/// - Generator string is "changeset-historic-api/0.1.0"
pub fn format_changesets_json(changesets: Vec<Changeset>) -> Result<String, serde_json::Error> {
    let json_changesets = changesets
        .into_iter()
        .map(|cs| ChangesetJson {
            id: cs.id,
            created_at: cs.created_at,
            closed_at: cs.closed_at,
            open: cs.open,
            user: cs.user,
            uid: cs.uid,
            min_lat: cs.min_lat,
            min_lon: cs.min_lon,
            max_lat: cs.max_lat,
            max_lon: cs.max_lon,
            num_changes: cs.num_changes,
            comments_count: cs.comments_count,
            tags: cs.description.map(|comment| Tags {
                comment: Some(comment),
            }),
        })
        .collect();

    let output = ChangesetOutput {
        version: "0.6".to_string(),
        generator: "changeset-historic-api/0.1.0".to_string(),
        changesets: json_changesets,
    };

    serde_json::to_string_pretty(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_format_empty_changesets() {
        let changesets = vec![];
        let result = format_changesets_json(changesets);
        assert!(result.is_ok());

        let json_str = result.unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["version"], "0.6");
        assert_eq!(parsed["generator"], "changeset-historic-api/0.1.0");
        assert_eq!(parsed["changesets"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_format_single_changeset() {
        let changesets = vec![Changeset {
            id: 168575829,
            created_at: Some(1729070400000), // 2024-10-16T08:00:00Z
            closed_at: Some(1729074000000),  // 2024-10-16T09:00:00Z
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
        }];

        let result = format_changesets_json(changesets);
        assert!(result.is_ok());

        let json_str = result.unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["version"], "0.6");
        assert_eq!(parsed["generator"], "changeset-historic-api/0.1.0");
        assert_eq!(parsed["changesets"].as_array().unwrap().len(), 1);

        let cs = &parsed["changesets"][0];
        assert_eq!(cs["id"], 168575829);
        assert_eq!(cs["open"], false);
        assert_eq!(cs["user"], "mvexel");
        assert_eq!(cs["uid"], 12345);
        assert_eq!(cs["num_changes"], 42);
        assert_eq!(cs["comments_count"], 2);
        assert_eq!(cs["tags"]["comment"], "Fixed street names in Manhattan");
        assert!(cs["created_at"].is_string());
        assert!(cs["closed_at"].is_string());
    }

    #[test]
    fn test_format_changeset_with_nulls() {
        let changesets = vec![Changeset {
            id: 123456,
            created_at: Some(1729070400000),
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
        }];

        let result = format_changesets_json(changesets);
        assert!(result.is_ok());

        let json_str = result.unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        let cs = &parsed["changesets"][0];
        assert_eq!(cs["id"], 123456);
        assert_eq!(cs["open"], true);
        assert!(cs["user"].is_null());
        assert!(cs["uid"].is_null());
        assert!(cs["min_lat"].is_null());
        assert!(cs["closed_at"].is_null());
        // tags should not be present if description is None
        assert!(cs["tags"].is_null());
    }

    #[test]
    fn test_multiple_changesets() {
        let changesets = vec![
            Changeset {
                id: 1,
                created_at: Some(1729070400000),
                closed_at: Some(1729074000000),
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
                created_at: Some(1729070400000),
                closed_at: None,
                open: true,
                user: Some("user2".to_string()),
                uid: Some(2),
                min_lat: None,
                min_lon: None,
                max_lat: None,
                max_lon: None,
                num_changes: 0,
                comments_count: 1,
                description: Some("Test changeset".to_string()),
            },
        ];

        let result = format_changesets_json(changesets);
        assert!(result.is_ok());

        let json_str = result.unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        let changesets_arr = parsed["changesets"].as_array().unwrap();
        assert_eq!(changesets_arr.len(), 2);

        assert_eq!(changesets_arr[0]["id"], 1);
        assert_eq!(changesets_arr[0]["user"], "user1");
        assert!(changesets_arr[0]["tags"].is_null());

        assert_eq!(changesets_arr[1]["id"], 2);
        assert_eq!(changesets_arr[1]["user"], "user2");
        assert_eq!(changesets_arr[1]["tags"]["comment"], "Test changeset");
    }
}
