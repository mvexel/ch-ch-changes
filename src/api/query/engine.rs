use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crate::api::models::Changeset;
use duckdb::Connection;
use std::path::{Path, PathBuf};

use super::partition::PartitionResolver;

/// Query filter options for changesets
#[derive(Debug, Clone, Default)]
pub struct ChangesetFilter {
    /// Filter by username
    pub user: Option<String>,
    /// Filter by user ID
    pub uid: Option<i64>,
    /// Filter by open/closed status (None = no filter, true = open only, false = closed only)
    pub open: Option<bool>,
    /// Maximum number of results (pagination)
    pub limit: Option<u32>,
    /// Number of results to skip (pagination)
    pub offset: Option<u32>,
}

/// DuckDB-based query engine for changeset Parquet files
pub struct QueryEngine {
    data_dir: PathBuf,
    conn: Connection,
}

impl QueryEngine {
    /// Create a new QueryEngine with a connection to the parquet files
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = PathBuf::from(data_dir.as_ref());

        // Verify data directory exists
        if !data_dir.exists() {
            return Err(anyhow::anyhow!(
                "Data directory does not exist: {}",
                data_dir.display()
            ));
        }

        // Create DuckDB connection (in-memory)
        let conn = Connection::open_in_memory()
            .context("Failed to create DuckDB connection")?;

        Ok(Self { data_dir, conn })
    }

    /// Get a single changeset by ID
    pub fn get_changeset_by_id(&self, id: i64) -> Result<Option<Changeset>> {
        let query = format!(
            "SELECT * FROM read_parquet('{}/**/*.parquet') WHERE id = {} LIMIT 1",
            self.data_dir.display(),
            id
        );

        let mut stmt = self.conn.prepare(&query)
            .context("Failed to prepare query")?;

        let rows = stmt.query_map([], |row| {
            Ok(Changeset {
                id: row.get(0)?,
                created_at: row.get(1)?,
                closed_at: row.get(2)?,
                open: row.get(3)?,
                user: row.get(4)?,
                uid: row.get(5)?,
                min_lat: row.get(6)?,
                min_lon: row.get(7)?,
                max_lat: row.get(8)?,
                max_lon: row.get(9)?,
                num_changes: row.get(10)?,
                comments_count: row.get(11)?,
                description: row.get(12)?,
            })
        }).context("Failed to execute query")?;

        for result in rows {
            return Ok(Some(result?));
        }

        Ok(None)
    }

    /// Get changesets within a date range (by created_at)
    pub fn get_changesets_by_date_range(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<Changeset>> {
        self.get_changesets_by_date_range_with_filter(start, end, ChangesetFilter::default())
    }

    /// Get changesets within a date range with optional filters
    pub fn get_changesets_by_date_range_with_filter(
        &self,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
        filter: ChangesetFilter,
    ) -> Result<Vec<Changeset>> {
        let start_millis = start.timestamp_millis();
        let end_millis = end.timestamp_millis();

        // Get relevant partitions to optimize queries
        let partitions = PartitionResolver::partitions_for_date_range(&self.data_dir, start, end)?;

        let glob_patterns = PartitionResolver::glob_pattern_for_partitions(&self.data_dir, &partitions);

        // If no partitions found, return empty result
        if glob_patterns.is_empty() {
            return Ok(Vec::new());
        }

        // Build file list for read_parquet
        let file_list = glob_patterns.join("', '");

        let mut query = format!(
            "SELECT * FROM read_parquet('{}', '{}') WHERE created_at >= {} AND created_at < {}",
            glob_patterns[0], file_list,
            start_millis, end_millis
        );

        // Apply filters
        if let Some(username) = &filter.user {
            query.push_str(&format!(" AND user = '{}'", username.replace("'", "''")));
        }
        if let Some(user_id) = filter.uid {
            query.push_str(&format!(" AND uid = {}", user_id));
        }
        if let Some(open_status) = filter.open {
            query.push_str(&format!(" AND open = {}", open_status));
        }

        // Add ordering
        query.push_str(" ORDER BY created_at DESC");

        // Add pagination
        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = filter.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        self.execute_changeset_query(&query)
    }

    /// Get changesets within an ID range
    pub fn get_changesets_by_id_range(
        &self,
        from_id: i64,
        to_id: i64,
    ) -> Result<Vec<Changeset>> {
        self.get_changesets_by_id_range_with_filter(from_id, to_id, ChangesetFilter::default())
    }

    /// Get changesets within an ID range with optional filters
    pub fn get_changesets_by_id_range_with_filter(
        &self,
        from_id: i64,
        to_id: i64,
        filter: ChangesetFilter,
    ) -> Result<Vec<Changeset>> {
        // For ID ranges, we need to query all partitions since IDs don't map to specific partitions
        let partitions = PartitionResolver::list_partitions(&self.data_dir)?;

        if partitions.is_empty() {
            return Ok(Vec::new());
        }

        let glob_patterns = PartitionResolver::glob_pattern_for_partitions(&self.data_dir, &partitions);
        let file_list = glob_patterns.join("', '");

        let mut query = format!(
            "SELECT * FROM read_parquet('{}', '{}') WHERE id >= {} AND id <= {}",
            glob_patterns[0], file_list,
            from_id, to_id
        );

        // Apply filters
        if let Some(username) = &filter.user {
            query.push_str(&format!(" AND user = '{}'", username.replace("'", "''")));
        }
        if let Some(user_id) = filter.uid {
            query.push_str(&format!(" AND uid = {}", user_id));
        }
        if let Some(open_status) = filter.open {
            query.push_str(&format!(" AND open = {}", open_status));
        }

        // Add ordering
        query.push_str(" ORDER BY id ASC");

        // Add pagination
        if let Some(limit) = filter.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = filter.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        self.execute_changeset_query(&query)
    }

    /// Get changesets by username
    pub fn get_changesets_by_user(&self, username: &str) -> Result<Vec<Changeset>> {
        let query = format!(
            "SELECT * FROM read_parquet('{}') WHERE user = '{}' ORDER BY created_at DESC LIMIT 1000",
            PartitionResolver::glob_pattern(&self.data_dir),
            username.replace("'", "''")
        );

        self.execute_changeset_query(&query)
    }

    /// Get changesets by user ID
    pub fn get_changesets_by_uid(&self, uid: i64) -> Result<Vec<Changeset>> {
        let query = format!(
            "SELECT * FROM read_parquet('{}') WHERE uid = {} ORDER BY created_at DESC LIMIT 1000",
            PartitionResolver::glob_pattern(&self.data_dir),
            uid
        );

        self.execute_changeset_query(&query)
    }

    /// Get only open changesets
    pub fn get_open_changesets(&self, limit: u32) -> Result<Vec<Changeset>> {
        let query = format!(
            "SELECT * FROM read_parquet('{}') WHERE open = true ORDER BY created_at DESC LIMIT {}",
            PartitionResolver::glob_pattern(&self.data_dir),
            limit
        );

        self.execute_changeset_query(&query)
    }

    /// Get only closed changesets
    pub fn get_closed_changesets(&self, limit: u32) -> Result<Vec<Changeset>> {
        let query = format!(
            "SELECT * FROM read_parquet('{}') WHERE open = false ORDER BY created_at DESC LIMIT {}",
            PartitionResolver::glob_pattern(&self.data_dir),
            limit
        );

        self.execute_changeset_query(&query)
    }

    /// Execute a raw SQL query and return changesets
    /// Useful for custom queries
    pub fn query(&self, sql: &str) -> Result<Vec<Changeset>> {
        self.execute_changeset_query(sql)
    }

    /// Count total changesets
    pub fn count_changesets(&self) -> Result<u64> {
        let query = format!(
            "SELECT COUNT(*) as cnt FROM read_parquet('{}') ",
            PartitionResolver::glob_pattern(&self.data_dir)
        );

        let mut stmt = self.conn.prepare(&query)
            .context("Failed to prepare count query")?;

        let count: u64 = stmt.query_row([], |row| row.get(0))
            .context("Failed to execute count query")?;

        Ok(count)
    }

    /// Count changesets matching a filter
    pub fn count_changesets_with_filter(&self, filter: &ChangesetFilter) -> Result<u64> {
        let mut query = format!(
            "SELECT COUNT(*) as cnt FROM read_parquet('{}') WHERE 1=1",
            PartitionResolver::glob_pattern(&self.data_dir)
        );

        if let Some(username) = &filter.user {
            query.push_str(&format!(" AND user = '{}'", username.replace("'", "''")));
        }
        if let Some(user_id) = filter.uid {
            query.push_str(&format!(" AND uid = {}", user_id));
        }
        if let Some(open_status) = filter.open {
            query.push_str(&format!(" AND open = {}", open_status));
        }

        let mut stmt = self.conn.prepare(&query)
            .context("Failed to prepare count query")?;

        let count: u64 = stmt.query_row([], |row| row.get(0))
            .context("Failed to execute count query")?;

        Ok(count)
    }

    // Private helper methods

    /// Execute a query that returns changesets
    fn execute_changeset_query(&self, query: &str) -> Result<Vec<Changeset>> {
        let mut stmt = self.conn.prepare(query)
            .context("Failed to prepare query")?;

        let rows = stmt.query_map([], |row| {
            Ok(Changeset {
                id: row.get(0)?,
                created_at: row.get(1)?,
                closed_at: row.get(2)?,
                open: row.get(3)?,
                user: row.get(4)?,
                uid: row.get(5)?,
                min_lat: row.get(6)?,
                min_lon: row.get(7)?,
                max_lat: row.get(8)?,
                max_lon: row.get(9)?,
                num_changes: row.get(10)?,
                comments_count: row.get(11)?,
                description: row.get(12)?,
            })
        }).context("Failed to execute changeset query")?;

        let mut changesets = Vec::new();
        for result in rows {
            changesets.push(result?);
        }

        Ok(changesets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_default() {
        let filter = ChangesetFilter::default();
        assert_eq!(filter.user, None);
        assert_eq!(filter.uid, None);
        assert_eq!(filter.open, None);
        assert_eq!(filter.limit, None);
        assert_eq!(filter.offset, None);
    }
}
