//! Partition resolution and selection
//!
//! Determines which Parquet partitions to query based on date/ID ranges.

use chrono::{DateTime, Datelike, NaiveDate};
use duckdb::Connection;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolves partition files based on query parameters
///
/// # Features
/// - Date to partition mapping
/// - ID range fallback mapping (returns all partitions)
/// - Partition filtering and pruning
pub struct PartitionResolver {
    root: PathBuf,
    partitions_by_key: HashMap<(u32, u32), PartitionInfo>,
    partitions_by_path: HashMap<String, PartitionInfo>,
    has_id_index: bool,
}

impl PartitionResolver {
    /// Create a new partition resolver
    ///
    /// # Arguments
    /// - `index_path` - Path to partitioned Parquet files
    pub fn new(index_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let root = PathBuf::from(index_path);
        let mut partitions_by_key = HashMap::new();
        let mut partitions_by_path = HashMap::new();
        let mut has_id_index = false;

        if let Some(partitions) = load_partition_stats_index(&root) {
            for info in partitions {
                has_id_index = true;
                partitions_by_key.insert((info.year, info.month), info.clone());
                partitions_by_path.insert(info.path.clone(), info);
            }
        } else {
            let files = list_parquet_files(&root);
            for path in files {
                let (year, month) = match parse_partition_key(&path) {
                    Some(value) => value,
                    None => continue,
                };

                let file_size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let info = PartitionInfo {
                    path: path.to_string_lossy().to_string(),
                    year,
                    month,
                    changeset_count: 0,
                    min_id: 0,
                    max_id: 0,
                    min_timestamp: 0,
                    max_timestamp: 0,
                    file_size_bytes,
                };

                partitions_by_key.insert((year, month), info.clone());
                partitions_by_path.insert(info.path.clone(), info);
            }
        }

        Ok(Self {
            root,
            partitions_by_key,
            partitions_by_path,
            has_id_index,
        })
    }

    /// Get partitions for a date range
    ///
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
            if let Some(info) = self.partitions_by_key.get(&(year, month)) {
                paths.push(info.path.clone());
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

    /// Get partitions for an ID range
    ///
    /// Uses the partition stats index when available, otherwise returns all partitions.
    pub fn get_partitions_for_id_range(
        &self,
        start_id: i64,
        end_id: i64,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        if start_id > end_id {
            return Err("start_id is after end_id".into());
        }

        if !self.has_id_index {
            return self.get_all_partitions();
        }

        let mut entries: Vec<_> = self.partitions_by_key.values().collect();
        entries.sort_by_key(|info| (info.year, info.month));
        let mut paths = Vec::new();

        for info in entries {
            if info.min_id <= end_id && info.max_id >= start_id {
                paths.push(info.path.clone());
            }
        }

        Ok(paths)
    }

    /// Get all available partitions
    ///
    pub fn get_all_partitions(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut entries: Vec<_> = self.partitions_by_key.values().collect();
        entries.sort_by_key(|info| (info.year, info.month));
        Ok(entries.iter().map(|info| info.path.clone()).collect())
    }

    /// Get partition for a specific date
    ///
    pub fn get_partition_for_date(&self, date: &str) -> Result<String, Box<dyn std::error::Error>> {
        let (year, month) = parse_date_year_month(date)?;
        self.partitions_by_key
            .get(&(year, month))
            .map(|info| info.path.clone())
            .ok_or_else(|| "partition not found".into())
    }

    /// Get partition containing a specific changeset ID
    ///
    /// Requires an ID range index to resolve a single partition.
    pub fn get_partition_for_id(&self, id: i64) -> Result<String, Box<dyn std::error::Error>> {
        if !self.has_id_index {
            return Err(format!("no ID range index available for id={id}").into());
        }

        for info in self.partitions_by_key.values() {
            if info.min_id <= id && info.max_id >= id {
                return Ok(info.path.clone());
            }
        }

        Err("partition not found".into())
    }

    /// Get metadata about a partition
    ///
    pub fn get_partition_metadata(
        &self,
        partition: &str,
    ) -> Result<PartitionInfo, Box<dyn std::error::Error>> {
        let path = Path::new(partition);
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        let key = full_path.to_string_lossy().to_string();

        self.partitions_by_path
            .get(&key)
            .cloned()
            .ok_or_else(|| "partition not found".into())
    }
}

/// Metadata about a partition.
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub path: String,
    pub year: u32,
    pub month: u32,
    pub changeset_count: u64,
    pub min_id: i64,
    pub max_id: i64,
    pub min_timestamp: i64,
    pub max_timestamp: i64,
    pub file_size_bytes: u64,
}

fn list_parquet_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_parquet_files(dir, &mut files);
    files
}

fn collect_parquet_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("indexes") {
                continue;
            }
            collect_parquet_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            files.push(path);
        }
    }
}

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

fn load_partition_stats_index(root: &Path) -> Option<Vec<PartitionInfo>> {
    let index_path = root.join("indexes/partition_stats.parquet");
    if !index_path.exists() {
        return None;
    }

    let conn = Connection::open_in_memory().ok()?;
    let index_sql = index_path.to_string_lossy().replace('\'', "''");
    let query = format!(
        "SELECT path, year, month, changeset_count, min_id, max_id, \
         min_created_at, max_created_at, file_size_bytes \
         FROM read_parquet('{}')",
        index_sql
    );

    let mut stmt = conn.prepare(&query).ok()?;
    let mut rows = stmt.query([]).ok()?;
    let mut partitions = Vec::new();

    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let path: String = row.get("path").ok()?;
                let year_val: i64 = row.get("year").ok()?;
                let month_val: i64 = row.get("month").ok()?;
                let count_val: i64 = row.get("changeset_count").ok()?;
                let min_id: i64 = row.get("min_id").ok()?;
                let max_id: i64 = row.get("max_id").ok()?;
                let min_ts: Option<i64> = row.get("min_created_at").ok()?;
                let max_ts: Option<i64> = row.get("max_created_at").ok()?;
                let file_size: i64 = row.get("file_size_bytes").ok()?;

                let year = u32::try_from(year_val).ok()?;
                let month = u32::try_from(month_val).ok()?;
                let changeset_count = u64::try_from(count_val).unwrap_or(0);
                let file_size_bytes = u64::try_from(file_size).unwrap_or(0);

                let full_path = if Path::new(&path).is_absolute() {
                    PathBuf::from(&path)
                } else {
                    root.join(&path)
                };

                partitions.push(PartitionInfo {
                    path: full_path.to_string_lossy().to_string(),
                    year,
                    month,
                    changeset_count,
                    min_id,
                    max_id,
                    min_timestamp: min_ts.unwrap_or(0),
                    max_timestamp: max_ts.unwrap_or(0),
                    file_size_bytes,
                });
            }
            Ok(None) => break,
            Err(_) => return None,
        }
    }

    Some(partitions)
}

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
    use duckdb::Connection;
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

    #[test]
    fn test_get_partition_for_date() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partition = resolver.get_partition_for_date("2024-05-15").expect("partition");
        assert!(partition.contains("month=05.parquet"));
    }

    #[test]
    fn test_get_partition_metadata() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partition = resolver.get_partition_for_date("20240601").expect("partition");
        let info = resolver
            .get_partition_metadata(&partition)
            .expect("metadata");
        assert_eq!(info.year, 2024);
        assert_eq!(info.month, 6);
        assert!(info.file_size_bytes > 0);
    }

    #[test]
    fn test_get_partitions_for_id_range_fallback() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver.get_partitions_for_id_range(1, 10).expect("range");
        assert_eq!(partitions.len(), 3);
    }

    #[test]
    fn test_id_range_with_index() {
        let dir = TempDir::new().expect("tempdir");
        let indexes_dir = dir.path().join("indexes");
        fs::create_dir_all(&indexes_dir).expect("indexes");

        let index_path = indexes_dir.join("partition_stats.parquet");
        let conn = Connection::open_in_memory().expect("duckdb");
        conn.execute_batch(
            "CREATE TABLE stats (
                path VARCHAR,
                year BIGINT,
                month BIGINT,
                changeset_count BIGINT,
                min_id BIGINT,
                max_id BIGINT,
                min_created_at BIGINT,
                max_created_at BIGINT,
                file_size_bytes BIGINT
            );",
        )
        .expect("create stats table");
        conn.execute_batch(
            "INSERT INTO stats VALUES
                ('year=2024/month=05.parquet', 2024, 5, 10, 1, 100, NULL, NULL, 1234),
                ('year=2024/month=06.parquet', 2024, 6, 10, 101, 200, NULL, NULL, 1234);",
        )
        .expect("insert stats");

        let index_path_sql = index_path.to_string_lossy().replace('\'', "''");
        let copy_sql = format!(
            "COPY stats TO '{}' (FORMAT PARQUET);",
            index_path_sql
        );
        conn.execute_batch(&copy_sql).expect("copy stats");

        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver
            .get_partitions_for_id_range(50, 80)
            .expect("range");
        assert_eq!(partitions.len(), 1);
        assert!(partitions[0].contains("month=05"));

        let partition = resolver.get_partition_for_id(150).expect("partition");
        assert!(partition.contains("month=06"));
    }
}
