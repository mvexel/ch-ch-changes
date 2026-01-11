use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use std::fs;
use std::path::{Path, PathBuf};

/// Represents a partition key for year/month-based partitioning
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PartitionKey {
    pub year: i32,
    pub month: u32,
}

impl PartitionKey {
    /// Create a partition key from a timestamp (in milliseconds since epoch)
    pub fn from_timestamp_millis(ts: i64) -> Self {
        let dt = DateTime::<Utc>::from_timestamp_millis(ts).unwrap_or_else(|| Utc::now());
        Self {
            year: dt.year(),
            month: dt.month(),
        }
    }

    /// Create a partition key from a DateTime
    pub fn from_datetime(dt: &DateTime<Utc>) -> Self {
        Self {
            year: dt.year(),
            month: dt.month(),
        }
    }

    /// Get the relative path for this partition (e.g., "year=2023/month=05.parquet")
    pub fn path(&self) -> String {
        format!("year={}/month={:02}.parquet", self.year, self.month)
    }

    /// Get the full file path for this partition
    pub fn full_path(&self, data_dir: &Path) -> PathBuf {
        data_dir.join(self.path())
    }
}

/// Helper functions for partition management
pub struct PartitionResolver;

impl PartitionResolver {
    /// Get all partition files that exist in the data directory
    pub fn list_partitions(data_dir: &Path) -> Result<Vec<PartitionKey>> {
        if !data_dir.exists() {
            return Err(anyhow::anyhow!(
                "Data directory does not exist: {}",
                data_dir.display()
            ));
        }

        let mut partitions = Vec::new();

        // Look for year=* directories
        for entry in fs::read_dir(data_dir)
            .with_context(|| format!("Failed to read directory: {}", data_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if !dir_name.starts_with("year=") {
                continue;
            }

            let year_str = &dir_name[5..]; // Skip "year="
            let year: i32 = year_str.parse()
                .ok()
                .ok_or_else(|| anyhow::anyhow!("Invalid year in partition: {}", year_str))?;

            // Look for month=*.parquet files in this directory
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                let file_path = entry.path();

                if !file_path.is_file() {
                    continue;
                }

                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                if !file_name.starts_with("month=") || !file_name.ends_with(".parquet") {
                    continue;
                }

                let month_str = &file_name[6..file_name.len() - 8]; // Skip "month=" and ".parquet"
                if let Ok(month) = month_str.parse::<u32>() {
                    if month > 0 && month <= 12 {
                        partitions.push(PartitionKey { year, month });
                    }
                }
            }
        }

        // Sort partitions by year and month
        partitions.sort();
        Ok(partitions)
    }

    /// Find partitions that overlap with a date range
    /// Returns all partitions where the entire month falls within [start, end)
    pub fn partitions_for_date_range(
        data_dir: &Path,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<PartitionKey>> {
        let all_partitions = Self::list_partitions(data_dir)?;

        let start_key = PartitionKey::from_datetime(start);
        let end_key = PartitionKey::from_datetime(end);

        Ok(all_partitions
            .into_iter()
            .filter(|pk| {
                // Include partition if it's within or overlaps the range
                if pk.year < start_key.year {
                    return false;
                }
                if pk.year > end_key.year {
                    return false;
                }
                if pk.year == start_key.year && pk.month < start_key.month {
                    return false;
                }
                if pk.year == end_key.year && pk.month > end_key.month {
                    return false;
                }
                true
            })
            .collect())
    }

    /// Find partitions that overlap with an ID range
    /// This is more complex as IDs don't correspond directly to partitions
    /// For simplicity, return all partitions (ID-based filtering happens in SQL)
    pub fn partitions_for_id_range(
        data_dir: &Path,
        _from_id: i64,
        _to_id: i64,
    ) -> Result<Vec<PartitionKey>> {
        // Since changesets can have arbitrary IDs from any time period,
        // we can't reliably determine which partitions contain a specific ID range
        // Just return all partitions and let the SQL query do the filtering
        Self::list_partitions(data_dir)
    }

    /// Build a SQL glob pattern to read all partition files
    /// Example: "data_dir/year=*/month=*.parquet"
    pub fn glob_pattern(data_dir: &Path) -> String {
        format!("{}/**/month=*.parquet", data_dir.display())
    }

    /// Build a SQL glob pattern for specific partitions
    pub fn glob_pattern_for_partitions(
        data_dir: &Path,
        partitions: &[PartitionKey],
    ) -> Vec<String> {
        partitions
            .iter()
            .map(|pk| pk.full_path(data_dir).to_string_lossy().to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_key_from_timestamp() {
        // 2023-05-15 12:00:00 UTC
        let ts = 1684161600000i64;
        let pk = PartitionKey::from_timestamp_millis(ts);
        assert_eq!(pk.year, 2023);
        assert_eq!(pk.month, 5);
    }

    #[test]
    fn test_partition_key_path() {
        let pk = PartitionKey {
            year: 2023,
            month: 5,
        };
        assert_eq!(pk.path(), "year=2023/month=05.parquet");
    }

    #[test]
    fn test_partition_key_ordering() {
        let pk1 = PartitionKey {
            year: 2023,
            month: 5,
        };
        let pk2 = PartitionKey {
            year: 2023,
            month: 6,
        };
        let pk3 = PartitionKey {
            year: 2024,
            month: 1,
        };

        assert!(pk1 < pk2);
        assert!(pk2 < pk3);
    }
}
