//! Metadata and health check route handlers
//!
//! Handles metadata endpoints that provide information about the dataset.
//!
//! # Endpoints
//! - `GET /health` - Health check endpoint
//! - `GET /meta/tip` - Get information about the latest data
//! - `GET /meta/coverage` - Get dataset coverage information
//!
//! # Responses
//! - 200 OK with JSON metadata
//! - 500 Internal Server Error if metadata unavailable

use crate::state::AppState;
use crate::API_VERSION;
use axum::{extract::State, Json};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub data_loaded: bool,
    pub partition_count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TipResponse {
    pub last_updated: String,
    pub latest_changeset_id: i64,
    pub earliest_changeset_id: i64,
    pub total_changesets: u64,
    pub data_through: String,
    pub update_frequency: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageResponse {
    pub temporal: TemporalCoverage,
    pub partitions: Vec<PartitionInfo>,
    pub total_changesets: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TemporalCoverage {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PartitionInfo {
    pub year: u32,
    pub month: u32,
    pub count: u64,
    pub file_size_mb: f64,
}

/// Health check endpoint
///
/// Returns basic health information about the API service.
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let parquet_files = list_parquet_files(state.engine.data_path());
    let partition_count = parquet_files.len() as u32;
    let data_loaded = partition_count > 0;

    Json(HealthResponse {
        status: "ok".to_string(),
        version: API_VERSION.to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        data_loaded,
        partition_count,
    })
}

/// Get tip/boundary information
///
/// Returns metadata about the latest changeset data available.
/// update_frequency is currently hardcoded.
pub async fn get_tip(State(state): State<AppState>) -> Json<TipResponse> {
    let parquet_files = list_parquet_files(state.engine.data_path());
    let last_updated = latest_modified_rfc3339(&parquet_files).unwrap_or_default();

    let latest_changeset_id = state.engine.get_latest_id().unwrap_or(0);
    let earliest_changeset_id = state.engine.get_earliest_id().unwrap_or(0);
    let total_changesets = state.engine.get_total_count().unwrap_or(0);
    let (_start, end) = state.engine.get_created_at_bounds().unwrap_or((None, None));

    Json(TipResponse {
        last_updated,
        latest_changeset_id,
        earliest_changeset_id,
        total_changesets,
        data_through: format_timestamp_millis(end),
        update_frequency: "weekly".to_string(),
    })
}

/// Get dataset coverage information
///
/// Returns detailed information about available partitions and temporal coverage.
pub async fn get_coverage(State(state): State<AppState>) -> Json<CoverageResponse> {
    let parquet_files = list_parquet_files(state.engine.data_path());
    let partition_counts = state.engine.get_partition_counts().unwrap_or_default();
    let partition_map = partition_counts
        .into_iter()
        .map(|entry| ((entry.year, entry.month), entry.count))
        .collect::<HashMap<_, _>>();

    let mut partitions: Vec<PartitionInfo> = parquet_files
        .iter()
        .filter_map(|path| build_partition_info(path, &partition_map))
        .collect();
    partitions.sort_by_key(|p| (p.year, p.month));

    let (start, end) = state.engine.get_created_at_bounds().unwrap_or((None, None));
    let total_changesets = state.engine.get_total_count().unwrap_or(0);

    Json(CoverageResponse {
        temporal: TemporalCoverage {
            start: format_timestamp_millis(start),
            end: format_timestamp_millis(end),
        },
        partitions,
        total_changesets,
    })
}

fn list_parquet_files(data_path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_parquet_files(data_path, &mut files);
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
            collect_parquet_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            files.push(path);
        }
    }
}

fn build_partition_info(
    path: &Path,
    partition_counts: &HashMap<(u32, u32), u64>,
) -> Option<PartitionInfo> {
    let (year, month) = parse_partition_key(path)?;
    let count = *partition_counts.get(&(year, month)).unwrap_or(&0);
    let size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file_size_mb = (size_bytes as f64) / (1024.0 * 1024.0);

    Some(PartitionInfo {
        year,
        month,
        count,
        file_size_mb,
    })
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

fn format_timestamp_millis(value: Option<i64>) -> String {
    match value {
        Some(ts) => Utc
            .timestamp_millis_opt(ts)
            .single()
            .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn latest_modified_rfc3339(files: &[PathBuf]) -> Option<String> {
    let latest = files
        .iter()
        .filter_map(|path| fs::metadata(path).ok()?.modified().ok())
        .max();

    latest.and_then(|ts| {
        let dt = ts
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single());
        dt.map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryEngine;
    use duckdb::Connection;
    use std::sync::Arc;
    use std::time::Instant;
    use tempfile::TempDir;

    fn setup_state() -> (TempDir, AppState) {
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
                description VARCHAR,
                tags MAP(VARCHAR, VARCHAR)
            );",
        )
        .expect("create table");

        conn.execute_batch(
            "INSERT INTO changesets VALUES
                (1, TIMESTAMP '2024-05-01 00:00:00', TIMESTAMP '2024-05-01 01:00:00', false, 'alice', 10, 1.0, 2.0, 3.0, 4.0, 5, 6, 'first', MAP {'comment': 'first'}),
                (2, TIMESTAMP '2024-05-02 12:00:00', NULL, true, 'bob', 20, 5.0, 6.0, 7.0, 8.0, 9, 0, 'second', MAP {'comment': 'second'});",
        )
        .expect("insert rows");

        let parquet_path_sql = parquet_path.to_string_lossy().replace('\'', "''");
        let copy_sql = format!(
            "COPY changesets TO '{}' (FORMAT PARQUET);",
            parquet_path_sql
        );
        conn.execute_batch(&copy_sql).expect("copy parquet");

        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
        };

        (dir, state)
    }

    fn setup_empty_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("tempdir");
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
        };

        (dir, state)
    }

    #[tokio::test]
    async fn test_health() {
        let (_dir, state) = setup_state();
        let response = health(State(state)).await.0;
        assert_eq!(response.status, "ok");
        assert!(response.data_loaded);
        assert_eq!(response.partition_count, 1);
    }

    #[tokio::test]
    async fn test_tip() {
        let (_dir, state) = setup_state();
        let response = get_tip(State(state)).await.0;
        assert_eq!(response.latest_changeset_id, 2);
        assert_eq!(response.earliest_changeset_id, 1);
        assert_eq!(response.total_changesets, 2);
        assert_eq!(response.data_through, "2024-05-02T12:00:00Z");
        assert!(!response.last_updated.is_empty());
    }

    #[tokio::test]
    async fn test_coverage() {
        let (_dir, state) = setup_state();
        let response = get_coverage(State(state)).await.0;
        assert_eq!(response.total_changesets, 2);
        assert_eq!(response.temporal.start, "2024-05-01T00:00:00Z");
        assert_eq!(response.temporal.end, "2024-05-02T12:00:00Z");
        assert_eq!(response.partitions.len(), 1);
        assert_eq!(response.partitions[0].year, 2024);
        assert_eq!(response.partitions[0].month, 5);
        assert_eq!(response.partitions[0].count, 2);
        assert!(response.partitions[0].file_size_mb > 0.0);
    }

    #[tokio::test]
    async fn test_health_empty() {
        let (_dir, state) = setup_empty_state();
        let response = health(State(state)).await.0;
        assert_eq!(response.status, "ok");
        assert!(!response.data_loaded);
        assert_eq!(response.partition_count, 0);
    }

    #[tokio::test]
    async fn test_tip_empty() {
        let (_dir, state) = setup_empty_state();
        let response = get_tip(State(state)).await.0;
        assert_eq!(response.latest_changeset_id, 0);
        assert_eq!(response.earliest_changeset_id, 0);
        assert_eq!(response.total_changesets, 0);
        assert_eq!(response.data_through, "");
        assert_eq!(response.last_updated, "");
    }

    #[tokio::test]
    async fn test_coverage_empty() {
        let (_dir, state) = setup_empty_state();
        let response = get_coverage(State(state)).await.0;
        assert_eq!(response.total_changesets, 0);
        assert_eq!(response.temporal.start, "");
        assert_eq!(response.temporal.end, "");
        assert!(response.partitions.is_empty());
    }
}
