//! SQL query builder
//!
//! Constructs SQL queries for DuckDB Parquet queries with proper partition selection.

use std::fmt::Write;

/// Query builder for constructing SQL queries
///
/// # Features
/// - Partition selection based on date/ID ranges
/// - WHERE clause building
/// - Query optimization hints
pub struct QueryBuilder {
    parquet_paths: Vec<String>,
    where_clauses: Vec<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    deduplicate: bool,
}

impl QueryBuilder {
    /// Create a new query builder
    pub fn new() -> Self {
        Self {
            parquet_paths: Vec::new(),
            where_clauses: Vec::new(),
            limit: None,
            offset: None,
            deduplicate: false,
        }
    }

    /// Select from specific Parquet file
    #[allow(dead_code)]
    pub fn from_parquet(mut self, path: &str) -> Self {
        self.parquet_paths = vec![path.to_string()];
        self
    }

    /// Select from multiple Parquet files (partitions)
    pub fn from_parquets(mut self, paths: Vec<&str>) -> Self {
        self.parquet_paths = paths.into_iter().map(|path| path.to_string()).collect();
        self
    }

    /// Add a WHERE clause
    #[allow(dead_code)]
    pub fn where_clause(mut self, condition: &str) -> Self {
        if !condition.trim().is_empty() {
            self.where_clauses.push(condition.trim().to_string());
        }
        self
    }

    /// Filter by changeset ID range
    pub fn filter_id_range(mut self, start_id: i64, end_id: i64) -> Self {
        self.where_clauses
            .push(format!("id >= {} AND id <= {}", start_id, end_id));
        self
    }

    /// Filter by date range
    pub fn filter_date_range(mut self, start_date: &str, end_date: &str) -> Self {
        let start = escape_sql_literal(start_date);
        let end = escape_sql_literal(end_date);
        self.where_clauses.push(format!(
            "created_at >= CAST('{}' AS TIMESTAMP) AND created_at <= CAST('{}' AS TIMESTAMP)",
            start, end
        ));
        self
    }

    /// Filter by username
    pub fn filter_user(mut self, username: &str) -> Self {
        let username = escape_sql_literal(username);
        self.where_clauses.push(format!("user = '{}'", username));
        self
    }

    /// Filter by user ID
    pub fn filter_uid(mut self, uid: i64) -> Self {
        self.where_clauses.push(format!("uid = {}", uid));
        self
    }

    /// Filter by open/closed status
    pub fn filter_open(mut self, open: bool) -> Self {
        self.where_clauses.push(format!("open = {}", open));
        self
    }

    /// Add LIMIT clause
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Add OFFSET clause
    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Enable deduplication (latest wins strategy)
    pub fn deduplicate(mut self) -> Self {
        self.deduplicate = true;
        self
    }

    /// Build the final SQL query string
    pub fn build(self) -> String {
        if self.parquet_paths.is_empty() {
            return String::new();
        }

        let source = build_parquet_source(&self.parquet_paths);
        // Use REPLACE to convert Map tags to JSON string for easier parsing
        let mut query = format!("SELECT * REPLACE (to_json(tags) AS tags) FROM {source}");

        if !self.where_clauses.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&self.where_clauses.join(" AND "));
        }

        if self.deduplicate {
            // Latest wins strategy: highest replication_sequence = freshest data
            query.push_str(" QUALIFY ROW_NUMBER() OVER (PARTITION BY id ORDER BY replication_sequence DESC NULLS LAST) = 1");
        }

        if let Some(limit) = self.limit {
            write!(&mut query, " LIMIT {}", limit).ok();
        }

        if let Some(offset) = self.offset {
            write!(&mut query, " OFFSET {}", offset).ok();
        }

        query
    }
}

fn build_parquet_source(paths: &[String]) -> String {
    if paths.len() == 1 {
        let path = escape_sql_literal(&paths[0]);
        return format!("read_parquet('{}', hive_partitioning=true)", path);
    }

    let mut list = String::new();
    for (idx, path) in paths.iter().enumerate() {
        if idx > 0 {
            list.push(',');
        }
        let escaped = escape_sql_literal(path);
        write!(&mut list, "'{}'", escaped).ok();
    }

    format!("read_parquet([{}], hive_partitioning=true)", list)
}

fn escape_sql_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_single_path() {
        let query = QueryBuilder::new()
            .from_parquet("/data/year=2024/month=05.parquet")
            .filter_user("alice")
            .limit(10)
            .build();
        assert_eq!(
            query,
            "SELECT * REPLACE (to_json(tags) AS tags) FROM read_parquet('/data/year=2024/month=05.parquet', hive_partitioning=true) WHERE user = 'alice' LIMIT 10"
        );
    }

    #[test]
    fn test_build_multiple_paths_and_filters() {
        let query = QueryBuilder::new()
            .from_parquets(vec!["/data/p1.parquet", "/data/p2.parquet"])
            .filter_id_range(1, 10)
            .filter_open(true)
            .offset(5)
            .build();
        assert_eq!(
            query,
            "SELECT * REPLACE (to_json(tags) AS tags) FROM read_parquet(['/data/p1.parquet','/data/p2.parquet'], hive_partitioning=true) WHERE id >= 1 AND id <= 10 AND open = true OFFSET 5"
        );
    }

    #[test]
    fn test_escape_literals() {
        let query = QueryBuilder::new()
            .from_parquet("/data/o'reilly.parquet")
            .filter_user("o'reilly")
            .build();
        assert_eq!(
            query,
            "SELECT * REPLACE (to_json(tags) AS tags) FROM read_parquet('/data/o''reilly.parquet', hive_partitioning=true) WHERE user = 'o''reilly'"
        );
    }
}
