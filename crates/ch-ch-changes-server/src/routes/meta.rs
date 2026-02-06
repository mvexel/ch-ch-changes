//! Metadata and health check route handlers
//!
//! Handles metadata endpoints that provide information about the dataset.
//!
//! # Endpoints
//! - `GET /health` - Health check endpoint
//! - `GET /meta/replication` - Get replication state info
//!
//! # Responses
//! - 200 OK with JSON metadata
//! - 500 Internal Server Error if metadata unavailable

use crate::models::ApiError;
use crate::models::error::ErrorResponse;
use crate::state::AppState;
use crate::API_VERSION;
use axum::{extract::State, Json};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub data_loaded: bool,
    pub partition_count: u32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReplicationStateResponse {
    pub sequence: u64,
    pub last_run: String,
    pub state_path: String,
}

/// Health check endpoint
///
/// Returns basic health information about the API service.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "API is healthy", body = HealthResponse)
    ),
    tag = "Meta"
)]
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

/// Get replication state information from the local state file.
#[utoipa::path(
    get,
    path = "/meta/state",
    responses(
        (status = 200, description = "Replication state information", body = ReplicationStateResponse),
        (status = 404, description = "Replication state not found", body = ErrorResponse),
        (status = 500, description = "Failed to read replication state", body = ErrorResponse)
    ),
    tag = "Meta"
)]
pub async fn get_replication_state(
    State(state): State<AppState>,
) -> Result<Json<ReplicationStateResponse>, ApiError> {
    let data_path = state.engine.data_path();
    let (state_path, content) = load_replication_state_file(data_path)
        .map_err(|e| ApiError::internal_error_with_details("Failed to read replication state", e))?
        .ok_or_else(|| ApiError::not_found("Replication state not found"))?;

    let (sequence, last_run) = parse_replication_state(&content).map_err(|e| {
        ApiError::internal_error_with_details("Failed to parse replication state", e)
    })?;

    Ok(Json(ReplicationStateResponse {
        sequence,
        last_run,
        state_path: state_path.to_string_lossy().to_string(),
    }))
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

fn load_replication_state_file(data_path: &Path) -> Result<Option<(PathBuf, String)>, String> {
    let candidates = [
        data_path.join("delta/state.yaml"),
        data_path.join("delta/state.txt"),
        data_path.join("state.yaml"),
        data_path.join("state.txt"),
    ];

    for path in candidates {
        if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            return Ok(Some((path, content)));
        }
    }

    Ok(None)
}

fn parse_replication_state(content: &str) -> Result<(u64, String), String> {
    let mut sequence: Option<u64> = None;
    let mut last_run: Option<DateTime<Utc>> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line == "---" {
            continue;
        }

        if let Some(value) = line.strip_prefix("sequence:") {
            sequence = Some(
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|e| format!("Invalid sequence: {}", e))?,
            );
        } else if let Some(value) = line.strip_prefix("last_run:") {
            last_run = Some(parse_replication_timestamp(value).map_err(|e| e.to_string())?);
        }
    }

    let sequence = sequence.ok_or_else(|| "Missing sequence in replication state".to_string())?;
    let last_run = last_run.ok_or_else(|| "Missing last_run in replication state".to_string())?;

    Ok((
        sequence,
        last_run.to_rfc3339_opts(SecondsFormat::Secs, true),
    ))
}

fn parse_replication_timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    let trimmed = value.trim();
    let formats = [
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M:%S%.f %z",
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%:z",
    ];

    for format in formats {
        if let Ok(parsed) = DateTime::parse_from_str(trimmed, format) {
            return Ok(parsed.with_timezone(&Utc));
        }
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(Utc.from_utc_datetime(&parsed));
    }

    DateTime::parse_from_rfc3339(trimmed).map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthConfig, AuthState};
    use crate::query::QueryEngine;
    use duckdb::Connection;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;
    use tempfile::TempDir;

    async fn setup_state() -> (TempDir, AppState) {
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
        let auth_state = AuthState::new(AuthConfig {
            oauth_client_id: Some("test".to_string()),
            oauth_client_secret: Some("test".to_string()),
            oauth_redirect_url: Some("http://localhost/auth/osm/callback".to_string()),
            oauth_authorize_url: "https://example.com".to_string(),
            oauth_token_url: "https://example.com".to_string(),
            oauth_user_details_url: "https://example.com".to_string(),
            oauth_scope: "read_prefs".to_string(),
            key_secret: Some("test-secret".to_string()),
            db_path: PathBuf::from(":memory:"),
            default_daily_limit: None,
            default_monthly_limit: None,
            require_auth: false,
        })
        .await
        .expect("auth");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
            auth: auth_state,
        };

        (dir, state)
    }

    async fn setup_empty_state() -> (TempDir, AppState) {
        let dir = TempDir::new().expect("tempdir");
        let engine = QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
        let auth_state = AuthState::new(AuthConfig {
            oauth_client_id: Some("test".to_string()),
            oauth_client_secret: Some("test".to_string()),
            oauth_redirect_url: Some("http://localhost/auth/osm/callback".to_string()),
            oauth_authorize_url: "https://example.com".to_string(),
            oauth_token_url: "https://example.com".to_string(),
            oauth_user_details_url: "https://example.com".to_string(),
            oauth_scope: "read_prefs".to_string(),
            key_secret: Some("test-secret".to_string()),
            db_path: PathBuf::from(":memory:"),
            default_daily_limit: None,
            default_monthly_limit: None,
            require_auth: false,
        })
        .await
        .expect("auth");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
            auth: auth_state,
        };

        (dir, state)
    }

    #[tokio::test]
    async fn test_health() {
        let (_dir, state) = setup_state().await;
        let response = health(State(state)).await.0;
        assert_eq!(response.status, "ok");
        assert!(response.data_loaded);
        assert_eq!(response.partition_count, 1);
    }

    #[tokio::test]
    async fn test_health_empty() {
        let (_dir, state) = setup_empty_state().await;
        let response = health(State(state)).await.0;
        assert_eq!(response.status, "ok");
        assert!(!response.data_loaded);
        assert_eq!(response.partition_count, 0);
    }
}
