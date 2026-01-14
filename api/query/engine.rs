//! DuckDB query execution engine
//!
//! Manages DuckDB connections and executes queries against Parquet files.

use crate::models::Changeset;
use crate::query::{PartitionResolver, QueryBuilder};
use duckdb::{
    params,
    types::{TimeUnit, Value},
    Connection,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Query engine for executing DuckDB queries
pub struct QueryEngine {
    conn: Arc<Mutex<Connection>>,
    data_path: PathBuf,
    partition_resolver: PartitionResolver,
}

#[derive(Debug, Clone)]
pub struct PartitionCount {
    pub year: u32,
    pub month: u32,
    pub count: u64,
}

impl QueryEngine {
    /// Create a new query engine
    ///
    /// # Arguments
    /// - `data_path` - Path to directory containing partitioned Parquet files
    pub fn new(data_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::open_in_memory()?;

        // Initialize parquet extension if needed (usually built-in)
        // conn.execute("INSTALL parquet; LOAD parquet;", [])?;

        let partition_resolver = PartitionResolver::new(data_path)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            data_path: PathBuf::from(data_path),
            partition_resolver,
        })
    }

    /// Helper to get the glob pattern for all parquet files
    fn get_glob_pattern(&self) -> String {
        self.data_path
            .join("year=*/**/*.parquet")
            .to_string_lossy()
            .to_string()
    }

    pub fn data_path(&self) -> &std::path::Path {
        &self.data_path
    }

    /// Query a single changeset by ID
    pub fn get_changeset(&self, id: i64) -> Result<Option<Changeset>, Box<dyn std::error::Error>> {
        if let Ok(partition) = self.partition_resolver.get_partition_for_id(id) {
            let query = QueryBuilder::new()
                .from_parquet(&partition)
                .filter_id_range(id, id)
                .build();
            let mut results = self.execute_query_sql(&query)?;
            return Ok(results.pop());
        }

        let conn = self.conn.lock().unwrap();
        let glob = self.get_glob_pattern();

        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM read_parquet('{}', hive_partitioning=true) WHERE id = ?",
            glob
        ))?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(serde_duckdb::from_row(&row)?))
        } else {
            Ok(None)
        }
    }

    /// Query changesets by date range
    pub fn get_changesets_by_date_range(
        &self,
        start_date: &str,
        end_date: &str,
        filters: &QueryFilters,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let start_ts = chrono::DateTime::parse_from_rfc3339(start_date)?;
        let end_ts = chrono::DateTime::parse_from_rfc3339(end_date)?;
        if start_ts > end_ts {
            return Ok(Vec::new());
        }

        let start_str = start_ts.naive_utc().format("%F %T").to_string();
        let end_str = end_ts.naive_utc().format("%F %T").to_string();

        let partitions = self
            .partition_resolver
            .get_partitions_for_date_range(start_date, end_date)?;
        if partitions.is_empty() {
            return Ok(Vec::new());
        }

        let mut builder = QueryBuilder::new()
            .from_parquets(partitions.iter().map(|p| p.as_str()).collect())
            .filter_date_range(&start_str, &end_str);
        builder = self.apply_filters_to_builder(builder, filters, true);
        let query = builder.build();
        self.execute_query_sql(&query)
    }

    /// Query changesets by ID range
    pub fn get_changesets_by_id_range(
        &self,
        start_id: i64,
        end_id: i64,
        filters: &QueryFilters,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        if start_id > end_id {
            return Ok(Vec::new());
        }

        let partitions = self
            .partition_resolver
            .get_partitions_for_id_range(start_id, end_id)?;
        if partitions.is_empty() {
            return Ok(Vec::new());
        }

        let mut builder = QueryBuilder::new()
            .from_parquets(partitions.iter().map(|p| p.as_str()).collect())
            .filter_id_range(start_id, end_id);
        builder = self.apply_filters_to_builder(builder, filters, true);
        let query = builder.build();
        self.execute_query_sql(&query)
    }

    /// Query changesets by user
    pub fn get_changesets_by_user(
        &self,
        username: &str,
        filters: &QueryFilters,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let mut builder = QueryBuilder::new()
            .from_parquet(&self.get_glob_pattern())
            .filter_user(username);
        builder = self.apply_filters_to_builder(builder, filters, false);
        let query = builder.build();
        self.execute_query_sql(&query)
    }

    /// Get the latest changeset ID available
    pub fn get_latest_id(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let glob = self.get_glob_pattern();

        let mut stmt = conn.prepare(&format!(
            "SELECT MAX(id) FROM read_parquet('{}', hive_partitioning=true)",
            glob
        ))?;

        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            let id: Option<i64> = row.get(0)?;
            Ok(id.unwrap_or(0))
        } else {
            Ok(0)
        }
    }

    /// Get total changeset count
    pub fn get_total_count(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let glob = self.get_glob_pattern();

        let mut stmt = conn.prepare(&format!(
            "SELECT COUNT(*) FROM read_parquet('{}', hive_partitioning=true)",
            glob
        ))?;

        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            let count: i64 = row.get(0)?;
            Ok(count as u64)
        } else {
            Ok(0)
        }
    }

    pub fn get_earliest_id(&self) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let glob = self.get_glob_pattern();

        let mut stmt = conn.prepare(&format!(
            "SELECT MIN(id) FROM read_parquet('{}', hive_partitioning=true)",
            glob
        ))?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let id: Option<i64> = row.get(0)?;
            Ok(id.unwrap_or(0))
        } else {
            Ok(0)
        }
    }

    pub fn get_created_at_bounds(
        &self,
    ) -> Result<(Option<i64>, Option<i64>), Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let glob = self.get_glob_pattern();

        let mut stmt = conn.prepare(&format!(
            "SELECT MIN(created_at) AS min_created_at, MAX(created_at) AS max_created_at \
             FROM read_parquet('{}', hive_partitioning=true)",
            glob
        ))?;

        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let min_val: Value = row.get("min_created_at")?;
            let max_val: Value = row.get("max_created_at")?;
            Ok((
                timestamp_millis_from_value(min_val),
                timestamp_millis_from_value(max_val),
            ))
        } else {
            Ok((None, None))
        }
    }

    pub fn get_partition_counts(&self) -> Result<Vec<PartitionCount>, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let files = list_parquet_files(self.data_path());
        let mut counts_map: std::collections::HashMap<(u32, u32), u64> =
            std::collections::HashMap::new();

        for file in files {
            let (year, month) = match parse_partition_key(&file) {
                Some(value) => value,
                None => continue,
            };

            let file_sql = file.to_string_lossy().replace('\'', "''");
            let query = format!("SELECT COUNT(*) FROM read_parquet('{}')", file_sql);
            let count: i64 = conn.query_row(&query, [], |row| row.get(0))?;

            let entry = counts_map.entry((year, month)).or_insert(0);
            *entry += count as u64;
        }

        Ok(counts_map
            .into_iter()
            .map(|((year, month), count)| PartitionCount { year, month, count })
            .collect())
    }

    fn apply_filters_to_builder(
        &self,
        mut builder: QueryBuilder,
        filters: &QueryFilters,
        include_username: bool,
    ) -> QueryBuilder {
        if include_username {
            if let Some(username) = &filters.username {
                builder = builder.filter_user(username);
            }
        }

        if let Some(uid) = filters.uid {
            builder = builder.filter_uid(uid);
        }

        if let Some(open) = filters.open {
            builder = builder.filter_open(open);
        }

        if let Some(limit) = filters.limit {
            builder = builder.limit(limit);
        }

        if let Some(offset) = filters.offset {
            builder = builder.offset(offset);
        }

        builder
    }

    /// Execute a query that returns a list of changesets
    fn execute_query_sql(
        &self,
        query: &str,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(query)?;
        let mut rows = stmt.query([])?;
        let mut changesets = Vec::new();

        while let Some(row) = rows.next()? {
            changesets.push(serde_duckdb::from_row(&row)?);
        }

        Ok(changesets)
    }
}

/// Query filters that can be applied to any query
#[derive(Debug, Clone, Default)]
pub struct QueryFilters {
    pub username: Option<String>,
    pub uid: Option<i64>,
    pub open: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl QueryFilters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_uid(mut self, uid: i64) -> Self {
        self.uid = Some(uid);
        self
    }

    pub fn with_open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }
}

fn timestamp_millis_from_value(value: Value) -> Option<i64> {
    match value {
        Value::Null => None,
        Value::Timestamp(unit, value) => Some(match unit {
            TimeUnit::Second => value.saturating_mul(1000),
            TimeUnit::Millisecond => value,
            TimeUnit::Microsecond => value / 1000,
            TimeUnit::Nanosecond => value / 1_000_000,
        }),
        Value::BigInt(value) => Some(value),
        Value::Int(value) => Some(value as i64),
        _ => None,
    }
}

fn partition_u32_from_value(value: Value) -> Option<u32> {
    match value {
        Value::Int(value) => u32::try_from(value).ok(),
        Value::BigInt(value) => u32::try_from(value).ok(),
        Value::UInt(value) => Some(value),
        Value::UBigInt(value) => u32::try_from(value).ok(),
        Value::Text(value) => value.parse::<u32>().ok(),
        _ => None,
    }
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
            // Skip the indexes directory
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

// Helper module to handle serde deserialization from DuckDB rows
mod serde_duckdb {
    use super::timestamp_millis_from_value;
    use crate::models::Changeset;
    use duckdb::{types::Value, Row};
    use std::collections::HashMap;

    pub fn from_row(row: &Row) -> Result<Changeset, duckdb::Error> {
        // Manually map fields because automated serde integration might be tricky with DuckDB's types
        // especially for the HashMap tags.
        // For now, let's try to map field by field.

        let id: i64 = row.get("id")?;
        let created_at = row
            .get::<_, Value>("created_at")
            .ok()
            .and_then(timestamp_millis_from_value);
        let closed_at = row
            .get::<_, Value>("closed_at")
            .ok()
            .and_then(timestamp_millis_from_value);
        let open: bool = row.get("open")?;
        let user: Option<String> = row.get("user").ok();
        let uid: Option<i64> = row.get("uid").ok();
        let min_lat: Option<f64> = row.get("min_lat").ok();
        let min_lon: Option<f64> = row.get("min_lon").ok();
        let max_lat: Option<f64> = row.get("max_lat").ok();
        let max_lon: Option<f64> = row.get("max_lon").ok();
        let num_changes: u32 = row.get::<_, i32>("num_changes")? as u32; // DuckDB might return i32
        let comments_count: u32 = row.get::<_, i32>("comments_count")? as u32;
        let description: Option<String> = row.get("description").ok();

        // Tags handling: DuckDB Map -> HashMap
        // FIXME: Implement tag parsing from DuckDB Value.
        // Current duckdb crate version's OrderedMap is opaque (private field) and doesn't implement IntoIterator.
        // We might need to cast to JSON in SQL or wait for upstream fix.
        let tags: HashMap<String, String> = HashMap::new();
        /*
        let tags_val: Value = row.get("tags").unwrap_or(Value::Null);
        let tags: HashMap<String, String> = match tags_val {
            Value::Map(entries) => {
                entries.0.into_iter().filter_map(|(k, v)| {
                    match (k, v) {
                        (Value::Text(k), Value::Text(v)) => Some((k, v)),
                        _ => None
                    }
                }).collect()
            },
            _ => HashMap::new(),
        };
        */

        Ok(Changeset {
            id,
            created_at,
            closed_at,
            open,
            user,
            uid,
            min_lat,
            min_lon,
            max_lat,
            max_lon,
            num_changes,
            comments_count,
            description,
            tags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use duckdb::Connection;
    use std::fs;
    use tempfile::TempDir;

    fn setup_parquet_dir() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let year_dir = dir.path().join("year=2024");
        fs::create_dir_all(&year_dir).expect("year dir");

        let parquet_path = year_dir.join("month=05.parquet");
        let conn = Connection::open_in_memory().expect("duckdb");

        conn.execute_batch(
            "CREATE TABLE changesets (
                id BIGINT,
                created_at TIMESTAMP,
                closed_at TIMESTAMP,
                open BOOLEAN,
                user VARCHAR,
                uid BIGINT,
                min_lat DOUBLE,
                min_lon DOUBLE,
                max_lat DOUBLE,
                max_lon DOUBLE,
                num_changes INTEGER,
                comments_count INTEGER,
                description VARCHAR
            );",
        )
        .expect("create table");

        conn.execute_batch(
            "INSERT INTO changesets VALUES
                (1, TIMESTAMP '2024-05-01 00:00:00', TIMESTAMP '2024-05-01 01:00:00', false, 'alice', 10, 1.0, 2.0, 3.0, 4.0, 5, 6, 'first'),
                (2, TIMESTAMP '2024-05-02 12:00:00', NULL, true, 'bob', 20, 5.0, 6.0, 7.0, 8.0, 9, 0, 'second');",
        )
        .expect("insert rows");

        let parquet_path_sql = parquet_path.to_string_lossy().replace('\'', "''");
        let copy_sql = format!(
            "COPY changesets TO '{}' (FORMAT PARQUET);",
            parquet_path_sql
        );
        conn.execute_batch(&copy_sql).expect("copy parquet");

        dir
    }

    #[test]
    fn test_get_changeset_by_id() {
        let dir = setup_parquet_dir();
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");

        let changeset = engine.get_changeset(1).expect("query").expect("row");
        assert_eq!(changeset.id, 1);
        assert_eq!(changeset.user.as_deref(), Some("alice"));

        let expected_created = Utc
            .with_ymd_and_hms(2024, 5, 1, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let expected_closed = Utc
            .with_ymd_and_hms(2024, 5, 1, 1, 0, 0)
            .unwrap()
            .timestamp_millis();

        assert_eq!(changeset.created_at, Some(expected_created));
        assert_eq!(changeset.closed_at, Some(expected_closed));
    }

    #[test]
    fn test_date_range_query() {
        let dir = setup_parquet_dir();
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");

        let results = engine
            .get_changesets_by_date_range(
                "2024-05-01T00:00:00Z",
                "2024-05-02T12:00:00Z",
                &QueryFilters::new(),
            )
            .expect("query");

        let ids: Vec<i64> = results.iter().map(|cs| cs.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }

    #[test]
    fn test_partition_counts_and_bounds() {
        let dir = setup_parquet_dir();
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");

        let counts = engine.get_partition_counts().expect("partition counts");
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].year, 2024);
        assert_eq!(counts[0].month, 5);
        assert_eq!(counts[0].count, 2);

        let (start, end) = engine.get_created_at_bounds().expect("bounds");
        let expected_start = Utc
            .with_ymd_and_hms(2024, 5, 1, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let expected_end = Utc
            .with_ymd_and_hms(2024, 5, 2, 12, 0, 0)
            .unwrap()
            .timestamp_millis();

        assert_eq!(start, Some(expected_start));
        assert_eq!(end, Some(expected_end));
    }
}
