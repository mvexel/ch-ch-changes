//! DuckDB query execution engine
//!
//! Manages DuckDB connections and executes queries against Parquet files.

use crate::models::Changeset;
use duckdb::{Connection, params};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

/// Query engine for executing DuckDB queries
pub struct QueryEngine {
    conn: Arc<Mutex<Connection>>,
    data_path: PathBuf,
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
        
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            data_path: PathBuf::from(data_path),
        })
    }

    /// Helper to get the glob pattern for all parquet files
    fn get_glob_pattern(&self) -> String {
        self.data_path.join("**/*.parquet").to_string_lossy().to_string()
    }

    /// Query a single changeset by ID
    pub fn get_changeset(&self, id: i64) -> Result<Option<Changeset>, Box<dyn std::error::Error>> {
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
        // TODO: Parse dates to timestamp integers for comparison if needed, 
        // but DuckDB handles string dates well if columns are timestamps.
        // Assuming columns are i64 (millis), we need to convert.
        let start_ts = chrono::DateTime::parse_from_rfc3339(start_date)?.timestamp_millis();
        let end_ts = chrono::DateTime::parse_from_rfc3339(end_date)?.timestamp_millis();

        let base_query = "SELECT * FROM read_parquet(?, hive_partitioning=true) WHERE created_at >= ? AND created_at <= ?";
        let (query, params) = self.apply_filters(base_query, filters);
        
        // Prepend the required params
        let mut final_params = Vec::new();
        final_params.push(duckdb::types::Value::Text(self.get_glob_pattern()));
        final_params.push(duckdb::types::Value::BigInt(start_ts));
        final_params.push(duckdb::types::Value::BigInt(end_ts));
        final_params.extend(params);

        self.execute_query(&query, final_params)
    }

    /// Query changesets by ID range
    pub fn get_changesets_by_id_range(
        &self,
        start_id: i64,
        end_id: i64,
        filters: &QueryFilters,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let base_query = "SELECT * FROM read_parquet(?, hive_partitioning=true) WHERE id >= ? AND id <= ?";
        let (query, params) = self.apply_filters(base_query, filters);

        let mut final_params = Vec::new();
        final_params.push(duckdb::types::Value::Text(self.get_glob_pattern()));
        final_params.push(duckdb::types::Value::BigInt(start_id));
        final_params.push(duckdb::types::Value::BigInt(end_id));
        final_params.extend(params);

        self.execute_query(&query, final_params)
    }

    /// Query changesets by user
    pub fn get_changesets_by_user(
        &self,
        username: &str,
        filters: &QueryFilters,
    ) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let base_query = "SELECT * FROM read_parquet(?, hive_partitioning=true) WHERE user = ?";
        let (query, params) = self.apply_filters(base_query, filters);

        let mut final_params = Vec::new();
        final_params.push(duckdb::types::Value::Text(self.get_glob_pattern()));
        final_params.push(duckdb::types::Value::Text(username.to_string()));
        final_params.extend(params);

        self.execute_query(&query, final_params)
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

    /// Apply filters to the query
    fn apply_filters(&self, base_query: &str, filters: &QueryFilters) -> (String, Vec<duckdb::types::Value>) {
        let mut query = base_query.to_string();
        let mut params = Vec::new();

        if let Some(username) = &filters.username {
            // Only add if not already in base query (simple check)
            if !base_query.contains("user = ?") {
                query.push_str(" AND user = ?");
                params.push(duckdb::types::Value::Text(username.clone()));
            }
        }

        if let Some(uid) = filters.uid {
            query.push_str(" AND uid = ?");
            params.push(duckdb::types::Value::BigInt(uid));
        }

        if let Some(open) = filters.open {
            query.push_str(" AND open = ?");
            params.push(duckdb::types::Value::Boolean(open));
        }

        // Limit and Offset
        if let Some(limit) = filters.limit {
            query.push_str(" LIMIT ?");
            params.push(duckdb::types::Value::BigInt(limit as i64));
        }

        if let Some(offset) = filters.offset {
            query.push_str(" OFFSET ?");
            params.push(duckdb::types::Value::BigInt(offset as i64));
        }

        (query, params)
    }

    /// Execute a query that returns a list of changesets
    fn execute_query(&self, query: &str, params: Vec<duckdb::types::Value>) -> Result<Vec<Changeset>, Box<dyn std::error::Error>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(query)?;
        
        // Convert Vec<Value> to slice of references which duckdb expects
        let params_refs: Vec<&dyn duckdb::ToSql> = params.iter().map(|v| v as &dyn duckdb::ToSql).collect();

        let mut rows = stmt.query(params_refs.as_slice())?;
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

// Helper module to handle serde deserialization from DuckDB rows
mod serde_duckdb {
    use crate::models::Changeset;
    use duckdb::{Row, types::Value};
    use std::collections::HashMap;
    
    pub fn from_row(row: &Row) -> Result<Changeset, duckdb::Error> {
        // Manually map fields because automated serde integration might be tricky with DuckDB's types
        // especially for the HashMap tags.
        // For now, let's try to map field by field.
        
        let id: i64 = row.get("id")?;
        let created_at: Option<i64> = row.get("created_at").ok();
        let closed_at: Option<i64> = row.get("closed_at").ok();
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
