//! Single changeset route handlers
//!
//! Handles requests for individual changesets by ID.

use crate::format::{json::JsonFormatter, xml::XmlFormatter, Formatter};
use crate::models::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct FormatQuery {
    format: Option<String>,
}

/// Get a single changeset by ID
pub async fn get_changeset(
    State(state): State<AppState>,
    Path(raw_id): Path<String>,
    Query(params): Query<FormatQuery>,
) -> Response {
    let (id, path_format) = match parse_id_and_format(&raw_id) {
        Ok(parsed) => parsed,
        Err(err) => return err.into_response(),
    };

    let format = path_format
        .as_deref()
        .or(params.format.as_deref())
        .unwrap_or("xml");
    match format {
        "json" => get_changeset_json(State(state), Path(id))
            .await
            .into_response(),
        _ => get_changeset_xml(State(state), Path(id))
            .await
            .into_response(),
    }
}

/// Get a single changeset in XML format
pub async fn get_changeset_xml(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    // Offload blocking DuckDB call to blocking thread pool
    let engine = state.engine.clone();
    let result = tokio::task::spawn_blocking(move || {
        engine.get_changeset(id).map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(Some(changeset)) => {
            let formatter = XmlFormatter::new();
            match formatter.format_changeset(&changeset) {
                Ok(xml) => ([(header::CONTENT_TYPE, "application/xml")], xml).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    ApiError::internal_error("Failed to format changeset").into_response()
                }
            }
        }
        Ok(None) => {
            ApiError::not_found(format!("Changeset {} not found", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

/// Get a single changeset in JSON format
pub async fn get_changeset_json(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    // Offload blocking DuckDB call to blocking thread pool
    let engine = state.engine.clone();
    let result = tokio::task::spawn_blocking(move || {
        engine.get_changeset(id).map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(Some(changeset)) => {
            let formatter = JsonFormatter::new(false);
            match formatter.format_changeset(&changeset) {
                Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    ApiError::internal_error("Failed to format changeset").into_response()
                }
            }
        }
        Ok(None) => {
            ApiError::not_found(format!("Changeset {} not found", id)).into_response()
        }
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

fn parse_id_and_format(raw: &str) -> Result<(i64, Option<String>), ApiError> {
    let (id_part, format) = match raw.rsplit_once('.') {
        Some((id_part, ext)) if ext.eq_ignore_ascii_case("json") => {
            (id_part, Some("json".to_string()))
        }
        Some((id_part, ext)) if ext.eq_ignore_ascii_case("xml") => {
            (id_part, Some("xml".to_string()))
        }
        _ => (raw, None),
    };

    let id = id_part.parse::<i64>().map_err(|_| {
        ApiError::bad_request_with_details(
            "Invalid changeset ID",
            format!("'{}' is not a valid changeset ID", id_part),
        )
    })?;
    Ok((id, format))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use duckdb::Connection;
    use serde_json::Value;
    use std::fs;
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

        let engine = crate::query::QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
        let state = AppState {
            engine: Arc::new(engine),
            started_at: Instant::now(),
        };

        (dir, state)
    }

    #[tokio::test]
    async fn test_get_changeset_json_suffix() {
        let (_dir, state) = setup_state();
        let response = get_changeset(
            State(state),
            Path("1.json".to_string()),
            Query(FormatQuery { format: None }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(content_type, Some("application/json"));

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["changeset"]["id"], 1);
        assert_eq!(value["changeset"]["tags"]["comment"], "first");
    }

    #[tokio::test]
    async fn test_get_changeset_xml_default() {
        let (_dir, state) = setup_state();
        let response = get_changeset(
            State(state),
            Path("1".to_string()),
            Query(FormatQuery { format: None }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(content_type, Some("application/xml"));

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8");
        assert!(body_str.contains("<changeset id=\"1\""));
    }
}
