//! Metadata and health check route handlers
//!
//! Handles metadata endpoints that provide information about the dataset.
//!
//! # Endpoints
//! - `GET /health` - Health check endpoint
//! - `GET /meta/latest` - Get the latest changeset info (cached)
//!
//! # Responses
//! - 200 OK with JSON metadata
//! - 500 Internal Server Error if metadata unavailable

use crate::models::ApiError;
use crate::state::{AppState, LatestChangesetCache};
use crate::API_VERSION;
use axum::{extract::State, Json};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
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
pub struct LatestChangesetResponse {
    pub latest_changeset_id: i64,
    pub created_at: String,
    pub closed_at: String,
    pub open: bool,
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

/// Get latest changeset information.
///
/// Cached on first request to avoid repeated expensive scans. Restart to refresh.
pub async fn get_latest(
    State(state): State<AppState>,
) -> Result<Json<LatestChangesetResponse>, ApiError> {
    if let Some(cached) = state.latest_changeset_cache.read().await.clone() {
        return Ok(Json(LatestChangesetResponse {
            latest_changeset_id: cached.latest_changeset_id,
            created_at: cached.created_at,
            closed_at: cached.closed_at,
            open: cached.open,
        }));
    }

    let latest_changeset_id = state.engine.get_latest_id().map_err(|e| {
        tracing::error!("Latest changeset id query failed: {}", e);
        ApiError::internal_error("Failed to query latest changeset")
    })?;

    let (created_at, closed_at, open) = if latest_changeset_id > 0 {
        match state.engine.get_changeset(latest_changeset_id) {
            Ok(Some(changeset)) => (
                format_timestamp_millis(changeset.created_at),
                format_timestamp_millis(changeset.closed_at),
                changeset.open,
            ),
            Ok(None) => (String::new(), String::new(), false),
            Err(e) => {
                tracing::error!("Latest changeset lookup failed: {}", e);
                return Err(ApiError::internal_error(
                    "Failed to query latest changeset",
                ));
            }
        }
    } else {
        (String::new(), String::new(), false)
    };

    let response = LatestChangesetResponse {
        latest_changeset_id,
        created_at,
        closed_at,
        open,
    };

    *state.latest_changeset_cache.write().await = Some(LatestChangesetCache {
        latest_changeset_id: response.latest_changeset_id,
        created_at: response.created_at.clone(),
        closed_at: response.closed_at.clone(),
        open: response.open,
    });

    Ok(Json(response))
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
            latest_changeset_cache: Arc::new(tokio::sync::RwLock::new(None)),
        };

        (dir, state)
    }

    fn setup_empty_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("tempdir");
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
            latest_changeset_cache: Arc::new(tokio::sync::RwLock::new(None)),
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
    async fn test_latest() {
        let (_dir, state) = setup_state();
        let response = get_latest(State(state)).await.expect("latest").0;
        assert_eq!(response.latest_changeset_id, 2);
        assert_eq!(response.created_at, "2024-05-02T12:00:00Z");
        assert_eq!(response.closed_at, "");
        assert!(response.open);
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
    async fn test_latest_empty() {
        let (_dir, state) = setup_empty_state();
        let response = get_latest(State(state)).await.expect("latest").0;
        assert_eq!(response.latest_changeset_id, 0);
        assert_eq!(response.created_at, "");
        assert_eq!(response.closed_at, "");
        assert!(!response.open);
    }
}
