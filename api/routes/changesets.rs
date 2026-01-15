//! Batch and range changeset route handlers

use crate::format::{json::JsonFormatter, jsonl::JsonLinesFormatter, xml::XmlFormatter, Formatter};
use crate::models::{ApiError, Changeset};
use crate::query::engine::QueryFilters;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use chrono::{NaiveDate, TimeZone, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub format: Option<String>,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub open: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl QueryParams {
    fn to_filters(&self) -> QueryFilters {
        let mut filters = QueryFilters::new();
        if let Some(user) = &self.user {
            filters = filters.with_username(user.clone());
        }
        if let Some(uid) = self.uid {
            filters = filters.with_uid(uid);
        }
        if let Some(open) = self.open {
            filters = filters.with_open(open);
        }
        if let Some(limit) = self.limit {
            filters = filters.with_limit(limit);
        }
        if let Some(offset) = self.offset {
            filters = filters.with_offset(offset);
        }
        filters
    }
}

fn extract_extension(s: &str) -> (&str, Option<String>) {
    if let Some(stripped) = s.strip_suffix(".json") {
        (stripped, Some("json".to_string()))
    } else if let Some(stripped) = s.strip_suffix(".jsonl") {
        (stripped, Some("jsonl".to_string()))
    } else if let Some(stripped) = s.strip_suffix(".xml") {
        (stripped, Some("xml".to_string()))
    } else {
        (s, None)
    }
}

fn format_response(changesets: &[Changeset], format: Option<&str>, pretty: bool) -> Response {
    match format {
        Some("json") => {
            let formatter = JsonFormatter::new(pretty);
            match formatter.format_changesets(changesets) {
                Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    ApiError::internal_error("Failed to format response").into_response()
                }
            }
        }
        Some("jsonl") | Some("jsonlines") => {
            let formatter = JsonLinesFormatter::new();
            match formatter.format_changesets(changesets) {
                Ok(jsonl) => {
                    ([(header::CONTENT_TYPE, "application/x-ndjson")], jsonl).into_response()
                }
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    ApiError::internal_error("Failed to format response").into_response()
                }
            }
        }
        _ => {
            let formatter = XmlFormatter::new();
            match formatter.format_changesets(changesets) {
                Ok(xml) => ([(header::CONTENT_TYPE, "application/xml")], xml).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    ApiError::internal_error("Failed to format response").into_response()
                }
            }
        }
    }
}

pub async fn get_changesets_by_day(
    State(state): State<AppState>,
    Path(date_str_raw): Path<String>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    let (date_str, ext_format) = extract_extension(&date_str_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    let date = match NaiveDate::parse_from_str(date_str, "%Y%m%d") {
        Ok(d) => d,
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid date format",
                format!("'{}' is not a valid date. Expected format: YYYYMMDD", date_str),
            )
            .into_response()
        }
    };

    let start = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59).unwrap());

    match state.engine.get_changesets_by_date_range(
        &start.to_rfc3339(),
        &end.to_rfc3339(),
        &params.to_filters(),
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

pub async fn get_changesets_by_id_range(
    State(state): State<AppState>,
    Path((start_str, end_raw)): Path<(String, String)>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    let (end_str, ext_format) = extract_extension(&end_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    let start_id = match start_str.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid start ID",
                format!("'{}' is not a valid changeset ID", start_str),
            )
            .into_response()
        }
    };

    let end_id = match end_str.parse::<i64>() {
        Ok(id) => id,
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid end ID",
                format!("'{}' is not a valid changeset ID", end_str),
            )
            .into_response()
        }
    };

    match state
        .engine
        .get_changesets_by_id_range(start_id, end_id, &params.to_filters())
    {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

pub async fn get_changesets_by_date_range(
    State(state): State<AppState>,
    Path((start_str, end_raw)): Path<(String, String)>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    let (end_str, ext_format) = extract_extension(&end_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    let start_date = match NaiveDate::parse_from_str(&start_str, "%Y%m%d") {
        Ok(d) => Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()),
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid start date format",
                format!("'{}' is not a valid date. Expected format: YYYYMMDD", start_str),
            )
            .into_response()
        }
    };

    let end_date = match NaiveDate::parse_from_str(&end_str, "%Y%m%d") {
        Ok(d) => Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap()),
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid end date format",
                format!("'{}' is not a valid date. Expected format: YYYYMMDD", end_str),
            )
            .into_response()
        }
    };

    match state.engine.get_changesets_by_date_range(
        &start_date.to_rfc3339(),
        &end_date.to_rfc3339(),
        &params.to_filters(),
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

pub async fn get_changesets_by_hour(
    State(state): State<AppState>,
    Path((date_str, hour_str_raw)): Path<(String, String)>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    let (hour_str, ext_format) = extract_extension(&hour_str_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    let date = match NaiveDate::parse_from_str(&date_str, "%Y%m%d") {
        Ok(d) => d,
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid date format",
                format!("'{}' is not a valid date. Expected format: YYYYMMDD", date_str),
            )
            .into_response()
        }
    };

    let hour = match hour_str.parse::<u32>() {
        Ok(h) if h < 24 => h,
        _ => {
            return ApiError::bad_request_with_details(
                "Invalid hour",
                format!("'{}' is not a valid hour. Must be 0-23", hour_str),
            )
            .into_response()
        }
    };

    let start_naive = match date.and_hms_opt(hour, 0, 0) {
        Some(dt) => dt,
        None => return ApiError::internal_error("Failed to construct timestamp").into_response(),
    };
    let end_naive = match date.and_hms_opt(hour, 59, 59) {
        Some(dt) => dt,
        None => return ApiError::internal_error("Failed to construct timestamp").into_response(),
    };

    let start = Utc.from_utc_datetime(&start_naive);
    let end = Utc.from_utc_datetime(&end_naive);

    match state.engine.get_changesets_by_date_range(
        &start.to_rfc3339(),
        &end.to_rfc3339(),
        &params.to_filters(),
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

pub async fn get_changesets_by_minute(
    State(state): State<AppState>,
    Path((date_str, hour_str, minute_str_raw)): Path<(String, String, String)>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    let (minute_str, ext_format) = extract_extension(&minute_str_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    let date = match NaiveDate::parse_from_str(&date_str, "%Y%m%d") {
        Ok(d) => d,
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid date format",
                format!("'{}' is not a valid date. Expected format: YYYYMMDD", date_str),
            )
            .into_response()
        }
    };

    let hour = match hour_str.parse::<u32>() {
        Ok(h) if h < 24 => h,
        _ => {
            return ApiError::bad_request_with_details(
                "Invalid hour",
                format!("'{}' is not a valid hour. Must be 0-23", hour_str),
            )
            .into_response()
        }
    };

    let minute = match minute_str.parse::<u32>() {
        Ok(m) if m < 60 => m,
        _ => {
            return ApiError::bad_request_with_details(
                "Invalid minute",
                format!("'{}' is not a valid minute. Must be 0-59", minute_str),
            )
            .into_response()
        }
    };

    let start_naive = match date.and_hms_opt(hour, minute, 0) {
        Some(dt) => dt,
        None => return ApiError::internal_error("Failed to construct timestamp").into_response(),
    };
    let end_naive = match date.and_hms_opt(hour, minute, 59) {
        Some(dt) => dt,
        None => return ApiError::internal_error("Failed to construct timestamp").into_response(),
    };

    let start = Utc.from_utc_datetime(&start_naive);
    let end = Utc.from_utc_datetime(&end_naive);

    match state.engine.get_changesets_by_date_range(
        &start.to_rfc3339(),
        &end.to_rfc3339(),
        &params.to_filters(),
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
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
    async fn test_get_changesets_by_day_json() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_day(
            State(state),
            Path("20240501".to_string()),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
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
        let changesets = value["changesets"].as_array().expect("changesets");
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0]["id"], 1);
    }

    #[tokio::test]
    async fn test_get_changesets_by_day_invalid_date() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_day(
            State(state),
            Path("bad-date".to_string()),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_hour_json() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_hour(
            State(state),
            Path(("20240502".to_string(), "12".to_string())),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        let changesets = value["changesets"].as_array().expect("changesets");
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0]["id"], 2);
    }

    #[tokio::test]
    async fn test_get_changesets_by_hour_invalid_hour() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_hour(
            State(state),
            Path(("20240502".to_string(), "24".to_string())),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_minute_json() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_minute(
            State(state),
            Path(("20240501".to_string(), "00".to_string(), "00".to_string())),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        let changesets = value["changesets"].as_array().expect("changesets");
        assert_eq!(changesets.len(), 1);
        assert_eq!(changesets[0]["id"], 1);
    }

    #[tokio::test]
    async fn test_get_changesets_by_minute_invalid_minute() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_minute(
            State(state),
            Path(("20240501".to_string(), "00".to_string(), "60".to_string())),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_id_range() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_id_range(
            State(state),
            Path(("1".to_string(), "2".to_string())),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        let changesets = value["changesets"].as_array().expect("changesets");
        assert_eq!(changesets.len(), 2);
    }

    #[tokio::test]
    async fn test_get_changesets_by_date_range() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_date_range(
            State(state),
            Path(("20240501".to_string(), "20240502".to_string())),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let value: Value = serde_json::from_slice(&body).expect("json");
        let changesets = value["changesets"].as_array().expect("changesets");
        assert_eq!(changesets.len(), 2);
    }

    #[tokio::test]
    async fn test_get_changesets_by_minute_jsonl_suffix() {
        let (_dir, state) = setup_state();
        let response = get_changesets_by_minute(
            State(state),
            Path((
                "20240501".to_string(),
                "00".to_string(),
                "00.jsonl".to_string(),
            )),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        assert_eq!(content_type, Some("application/x-ndjson"));
    }
}
