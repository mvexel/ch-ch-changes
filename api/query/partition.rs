//! Partition resolution and selection
//!
//! Determines which Parquet partitions to query based on date ranges.
//! Uses filesystem paths only - no index files required.

use chrono::{DateTime, Datelike, NaiveDate};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolves partition files based on query parameters
pub struct PartitionResolver {
    root: PathBuf,
    partitions_by_key: HashMap<(u32, u32), Vec<PathBuf>>,
}

impl PartitionResolver {
    /// Create a new partition resolver
    ///
    /// # Arguments
    /// - `data_path` - Path to directory containing partitioned Parquet files
    pub fn new(data_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let root = PathBuf::from(data_path);
        let mut partitions_by_key: HashMap<(u32, u32), Vec<PathBuf>> = HashMap::new();

        // Check for base/ and delta/ directories
        let base = root.join("base");
        let delta = root.join("delta");
        let roots = if base.exists() || delta.exists() {
            let mut roots = Vec::new();
            if base.exists() {
                roots.push(base);
            }
            if delta.exists() {
                roots.push(delta);
            }
            roots
        } else {
            vec![root.clone()]
        };

        // Scan directories and discover partitions from filesystem
        for root_dir in roots {
            let files = list_parquet_files(&root_dir);
            for path in files {
                if let Some((year, month)) = parse_partition_key(&path) {
                    partitions_by_key
                        .entry((year, month))
                        .or_default()
                        .push(path);
                }
            }
        }

        Ok(Self {
            root,
            partitions_by_key,
        })
    }

    /// Get partitions for a date range
    ///
    /// Maps the date range to (year, month) tuples and returns all files
    /// for those months from both base and delta directories.
    pub fn get_partitions_for_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let (start_year, start_month) = parse_date_year_month(start_date)?;
        let (end_year, end_month) = parse_date_year_month(end_date)?;

        if (start_year, start_month) > (end_year, end_month) {
            return Err("start_date is after end_date".into());
        }

        let mut paths = Vec::new();
        let mut year = start_year;
        let mut month = start_month;

        loop {
            if let Some(partition_paths) = self.partitions_by_key.get(&(year, month)) {
                paths.extend(partition_paths.iter().map(|p| p.to_string_lossy().to_string()));
            }

            if (year, month) == (end_year, end_month) {
                break;
            }

            if month == 12 {
                year += 1;
                month = 1;
            } else {
                month += 1;
            }
        }

        Ok(paths)
    }

    /// Get all available partitions sorted by (year, month)
    pub fn get_all_partitions(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut entries: Vec<_> = self.partitions_by_key.iter().collect();
        entries.sort_by_key(|(key, _)| *key);

        let mut paths = Vec::new();
        for (_, partition_paths) in entries {
            paths.extend(partition_paths.iter().map(|p| p.to_string_lossy().to_string()));
        }

        Ok(paths)
    }
}

/// Recursively list all parquet files in a directory
fn list_parquet_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_parquet_files(dir, &mut files);
    files
}

/// Helper to recursively collect parquet files
fn collect_parquet_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip indexes directory
            if path.file_name().and_then(|n| n.to_str()) == Some("indexes") {
                continue;
            }
            collect_parquet_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            files.push(path);
        }
    }
}

/// Parse year and month from a partition path
///
/// Expects paths like: .../year=2024/month=05.parquet or .../year=2024/month=05/file.parquet
fn parse_partition_key(path: &Path) -> Option<(u32, u32)> {
    let mut year: Option<u32> = None;
    let mut month: Option<u32> = None;

    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if let Some(value) = part.strip_prefix("year=") {
            year = value.parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("month=") {
            let value = value.trim_end_matches(".parquet");
            month = value.parse::<u32>().ok();
        }
    }

    match (year, month) {
        (Some(year), Some(month)) => Some((year, month)),
        _ => None,
    }
}

/// Parse a date string into (year, month)
fn parse_date_year_month(date: &str) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let parsed_date = if let Ok(value) = NaiveDate::parse_from_str(date, "%Y%m%d") {
        value
    } else if let Ok(value) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        value
    } else if let Ok(value) = DateTime::parse_from_rfc3339(date) {
        value.naive_utc().date()
    } else {
        return Err("unsupported date format".into());
    };

    let year = u32::try_from(parsed_date.year())
        .map_err(|_| "unsupported year for partition")?;
    Ok((year, parsed_date.month()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_partitions() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let year_2023 = dir.path().join("year=2023");
        let year_2024 = dir.path().join("year=2024");
        fs::create_dir_all(&year_2023).expect("year dir");
        fs::create_dir_all(&year_2024).expect("year dir");

        fs::write(year_2023.join("month=12.parquet"), b"data").expect("write 2023-12");
        fs::write(year_2024.join("month=05.parquet"), b"data").expect("write 2024-05");
        fs::write(year_2024.join("month=06.parquet"), b"data").expect("write 2024-06");

        dir
    }

    #[test]
    fn test_get_all_partitions_sorted() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver.get_all_partitions().expect("partitions");
        assert_eq!(partitions.len(), 3);
        assert!(partitions[0].contains("year=2023"));
        assert!(partitions[1].contains("year=2024"));
    }

    #[test]
    fn test_get_partitions_for_date_range() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver
            .get_partitions_for_date_range("20240501", "20240630")
            .expect("range");
        assert_eq!(partitions.len(), 2);
        assert!(partitions[0].contains("month=05"));
        assert!(partitions[1].contains("month=06"));
    }
}
