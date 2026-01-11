use serde::{Deserialize, Serialize};

/// Metadata about the tip of the changeset data (latest state)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipMetadata {
    /// RFC3339 formatted timestamp of when the metadata was last updated
    pub last_updated: String,

    /// The ID of the latest changeset in the dataset
    pub latest_changeset_id: i64,

    /// RFC3339 formatted timestamp when the latest changeset was created
    pub latest_changeset_created_at: String,

    /// Total number of changesets in the dataset
    pub total_changesets: i64,

    /// Total number of unique users
    pub total_users: i64,

    /// Number of open (unfinished) changesets
    pub open_changesets_count: i64,

    /// Date range covered by the dataset (ISO 8601 format: YYYY-MM-DD)
    pub coverage_start_date: String,

    /// Date range covered by the dataset (ISO 8601 format: YYYY-MM-DD)
    pub coverage_end_date: String,
}

/// Metadata about the temporal and partition coverage of the dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageMetadata {
    /// Earliest changeset creation date in the dataset (ISO 8601 format)
    pub earliest_date: String,

    /// Latest changeset creation date in the dataset (ISO 8601 format)
    pub latest_date: String,

    /// List of available partitions (organized by year/month)
    pub partitions: Vec<PartitionInfo>,

    /// Total size of all partition files in MB
    pub total_size_mb: f64,

    /// Number of changesets in the dataset
    pub total_changesets: i64,
}

/// Information about a single partition (typically organized by year/month)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionInfo {
    /// Year of the partition
    pub year: u16,

    /// Month of the partition (1-12), or 0 if not month-partitioned
    #[serde(skip_serializing_if = "is_zero")]
    pub month: u8,

    /// Number of changesets in this partition
    pub count: u32,

    /// File size in megabytes
    pub file_size_mb: f64,

    /// Path to the partition file (relative to base URL)
    pub path: String,

    /// Min changeset ID in this partition
    pub min_id: i64,

    /// Max changeset ID in this partition
    pub max_id: i64,

    /// Minimum timestamp in this partition (milliseconds since epoch)
    pub min_timestamp: i64,

    /// Maximum timestamp in this partition (milliseconds since epoch)
    pub max_timestamp: i64,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

impl PartitionInfo {
    /// Create a new PartitionInfo for a year-only partition
    pub fn new_year(
        year: u16,
        count: u32,
        file_size_mb: f64,
        path: String,
        min_id: i64,
        max_id: i64,
        min_timestamp: i64,
        max_timestamp: i64,
    ) -> Self {
        Self {
            year,
            month: 0,
            count,
            file_size_mb,
            path,
            min_id,
            max_id,
            min_timestamp,
            max_timestamp,
        }
    }

    /// Create a new PartitionInfo for a year/month partition
    pub fn new_month(
        year: u16,
        month: u8,
        count: u32,
        file_size_mb: f64,
        path: String,
        min_id: i64,
        max_id: i64,
        min_timestamp: i64,
        max_timestamp: i64,
    ) -> Self {
        Self {
            year,
            month,
            count,
            file_size_mb,
            path,
            min_id,
            max_id,
            min_timestamp,
            max_timestamp,
        }
    }

    /// Get a human-readable name for the partition
    pub fn name(&self) -> String {
        if self.month == 0 {
            format!("{}", self.year)
        } else {
            format!("{:04}-{:02}", self.year, self.month)
        }
    }
}

/// Statistics about a specific user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStats {
    /// User ID
    pub uid: i64,

    /// Username
    pub username: String,

    /// Total number of changesets created by this user
    pub changeset_count: u32,

    /// Total number of changes (objects modified) across all changesets
    pub total_changes: u64,

    /// Date of first changeset (ISO 8601 format)
    pub first_changeset_date: String,

    /// Date of most recent changeset (ISO 8601 format)
    pub last_changeset_date: String,

    /// Number of comments left on changesets by this user
    pub total_comments: u32,

    /// Total bounding box coverage (merged bbox of all changesets)
    pub bbox: Option<BoundingBox>,
}

/// A geographic bounding box
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    /// Minimum latitude
    pub min_lat: f64,

    /// Minimum longitude
    pub min_lon: f64,

    /// Maximum latitude
    pub max_lat: f64,

    /// Maximum longitude
    pub max_lon: f64,
}

impl BoundingBox {
    /// Create a new bounding box
    pub fn new(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> Self {
        Self {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        }
    }

    /// Calculate the area of the bounding box in square degrees
    pub fn area(&self) -> f64 {
        (self.max_lat - self.min_lat) * (self.max_lon - self.min_lon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_name_year() {
        let partition = PartitionInfo::new_year(2023, 100, 10.5, "2023.parquet".to_string(), 1, 100, 0, 100);
        assert_eq!(partition.name(), "2023");
    }

    #[test]
    fn test_partition_name_month() {
        let partition = PartitionInfo::new_month(
            2023,
            5,
            100,
            10.5,
            "2023-05.parquet".to_string(),
            1,
            100,
            0,
            100,
        );
        assert_eq!(partition.name(), "2023-05");
    }

    #[test]
    fn test_bbox_area() {
        let bbox = BoundingBox::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(bbox.area(), 1.0);
    }

    #[test]
    fn test_bbox_large_area() {
        let bbox = BoundingBox::new(-90.0, -180.0, 90.0, 180.0);
        assert_eq!(bbox.area(), 64800.0);
    }
}
