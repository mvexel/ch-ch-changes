//! Batch and range changeset route handlers

use crate::format::{
    json::JsonFormatter, jsonl::JsonLinesFormatter, xml::XmlFormatter, Formatter,
    StreamingFormatter,
};
use crate::models::{ApiError, Changeset};
use crate::query::engine::QueryFilters;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use chrono::{NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use std::convert::Infallible;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub format: Option<String>,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub open: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Request streaming response mode (currently disabled)
    pub stream: Option<bool>,
    pub order_by: Option<String>,
    pub order_direction: Option<String>,
}

// Streaming is disabled due to severe performance/oom issues under load.
// Keep the code paths in place for a future revisit with proper isolation.
const STREAMING_ENABLED: bool = true;

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
        if let Some(order_by) = &self.order_by {
            filters = filters.with_order_by(order_by.clone(), self.order_direction.clone());
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

/// Create a streaming HTTP response from a channel of changesets
fn create_streaming_response(
    format: Option<&str>,
    rx: tokio::sync::mpsc::Receiver<Changeset>,
) -> Response {
    // Determine content type and create appropriate stream based on format
    match format {
        Some("json") => {
            let formatter = JsonFormatter::new(false);
            let content_type = formatter.content_type();
            let stream = create_format_stream(formatter, rx);
            Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from_stream(stream))
                .unwrap()
        }
        Some("jsonl") | Some("jsonlines") => {
            let formatter = JsonLinesFormatter::new();
            let content_type = formatter.content_type();
            let stream = create_format_stream(formatter, rx);
            Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from_stream(stream))
                .unwrap()
        }
        _ => {
            let formatter = XmlFormatter::new();
            let content_type = formatter.content_type();
            let stream = create_format_stream(formatter, rx);
            Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from_stream(stream))
                .unwrap()
        }
    }
}

fn streaming_disabled_response() -> Response {
    ApiError::bad_request_with_details(
        "Streaming disabled",
        "Streaming responses are temporarily disabled due to performance issues under load.",
    )
    .into_response()
}

/// Create an async stream that formats changesets using the given formatter
fn create_format_stream<F: StreamingFormatter + 'static>(
    formatter: F,
    rx: tokio::sync::mpsc::Receiver<Changeset>,
) -> impl futures_core::Stream<Item = Result<String, Infallible>> {
    async_stream::stream! {
        // Emit header
        yield Ok::<_, Infallible>(formatter.stream_header());

        let mut is_first = true;
        let mut rx = rx;

        while let Some(changeset) = rx.recv().await {
            match formatter.stream_item(&changeset, is_first) {
                Ok(item) => {
                    yield Ok(item);
                    is_first = false;
                }
                Err(e) => {
                    tracing::error!("Streaming format error: {}", e);
                    // Continue processing remaining items
                }
            }
        }

        // Emit footer
        yield Ok(formatter.stream_footer());
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
                format!(
                    "'{}' is not a valid date. Expected format: YYYYMMDD",
                    date_str
                ),
            )
            .into_response()
        }
    };

    let start = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59).unwrap());

    // Streaming mode
    if params.stream.unwrap_or(false) {
        // if !STREAMING_ENABLED {
        //     return streaming_disabled_response();
        // }
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let engine = state.engine.clone();
        let start_str = start.to_rfc3339();
        let end_str = end.to_rfc3339();
        let filters = params.to_filters();

        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                engine.stream_changesets_by_date_range(&start_str, &end_str, &filters, tx)
            {
                tracing::error!("Streaming query error: {}", e);
            }
        });

        return create_streaming_response(params.format.as_deref(), rx);
    }

    // Non-streaming mode - offload to blocking thread pool
    let engine = state.engine.clone();
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();
    let filters = params.to_filters();
    let format_opt = params.format.clone();

    let result = tokio::task::spawn_blocking(move || {
        engine
            .get_changesets_by_date_range(&start_str, &end_str, &filters)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(changesets) => format_response(&changesets, format_opt.as_deref(), false),
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
                format!(
                    "'{}' is not a valid date. Expected format: YYYYMMDD",
                    start_str
                ),
            )
            .into_response()
        }
    };

    let end_date = match NaiveDate::parse_from_str(end_str, "%Y%m%d") {
        Ok(d) => Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap()),
        Err(_) => {
            return ApiError::bad_request_with_details(
                "Invalid end date format",
                format!(
                    "'{}' is not a valid date. Expected format: YYYYMMDD",
                    end_str
                ),
            )
            .into_response()
        }
    };

    // Safety limit: Maximum 1-day date range to prevent OOM
    const MAX_DAYS: i64 = 1;
    let days_diff = (end_date - start_date).num_days();
    if days_diff > MAX_DAYS {
        return ApiError::bad_request_with_details(
            "Date range too large",
            format!(
                "Requested {} days, but maximum allowed is {} days. Please use a smaller date range or pagination.",
                days_diff, MAX_DAYS
            ),
        )
        .into_response();
    }

    // Streaming mode
    if params.stream.unwrap_or(false) {
        // if !STREAMING_ENABLED {
        //     return streaming_disabled_response();
        // }
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let engine = state.engine.clone();
        let start = start_date.to_rfc3339();
        let end = end_date.to_rfc3339();
        let filters = params.to_filters();

        tokio::task::spawn_blocking(move || {
            if let Err(e) = engine.stream_changesets_by_date_range(&start, &end, &filters, tx) {
                tracing::error!("Streaming query error: {}", e);
            }
        });

        return create_streaming_response(params.format.as_deref(), rx);
    }

    // Non-streaming mode - offload to blocking thread pool
    let engine = state.engine.clone();
    let start_str = start_date.to_rfc3339();
    let end_str = end_date.to_rfc3339();
    let filters = params.to_filters();
    let format_opt = params.format.clone();

    let result = tokio::task::spawn_blocking(move || {
        engine
            .get_changesets_by_date_range(&start_str, &end_str, &filters)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(changesets) => {
            // Safety limit: Check result set size to prevent OOM
            const MAX_RESULT_SIZE: usize = 100_000;
            if changesets.len() > MAX_RESULT_SIZE {
                return ApiError::bad_request_with_details(
                    "Result set too large",
                    format!(
                        "Query returned {} changesets, but maximum allowed is {}. Please use a smaller date range, add filters (user, uid, open), or use pagination (limit/offset).",
                        changesets.len(), MAX_RESULT_SIZE
                    ),
                )
                .into_response();
            }
            format_response(&changesets, format_opt.as_deref(), false)
        }
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
                format!(
                    "'{}' is not a valid date. Expected format: YYYYMMDD",
                    date_str
                ),
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

    // Streaming mode
    if params.stream.unwrap_or(false) {
        // if !STREAMING_ENABLED {
        //     return streaming_disabled_response();
        // }
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let engine = state.engine.clone();
        let start_str = start.to_rfc3339();
        let end_str = end.to_rfc3339();
        let filters = params.to_filters();

        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                engine.stream_changesets_by_date_range(&start_str, &end_str, &filters, tx)
            {
                tracing::error!("Streaming query error: {}", e);
            }
        });

        return create_streaming_response(params.format.as_deref(), rx);
    }

    // Non-streaming mode - offload to blocking thread pool
    let engine = state.engine.clone();
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();
    let filters = params.to_filters();
    let format_opt = params.format.clone();

    let result = tokio::task::spawn_blocking(move || {
        engine
            .get_changesets_by_date_range(&start_str, &end_str, &filters)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(changesets) => format_response(&changesets, format_opt.as_deref(), false),
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
                format!(
                    "'{}' is not a valid date. Expected format: YYYYMMDD",
                    date_str
                ),
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

    // Streaming mode
    if params.stream.unwrap_or(false) {
        // if !STREAMING_ENABLED {
        //     return streaming_disabled_response();
        // }
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let engine = state.engine.clone();
        let start_str = start.to_rfc3339();
        let end_str = end.to_rfc3339();
        let filters = params.to_filters();

        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                engine.stream_changesets_by_date_range(&start_str, &end_str, &filters, tx)
            {
                tracing::error!("Streaming query error: {}", e);
            }
        });

        return create_streaming_response(params.format.as_deref(), rx);
    }

    // Non-streaming mode - offload to blocking thread pool
    let engine = state.engine.clone();
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();
    let filters = params.to_filters();
    let format_opt = params.format.clone();

    let result = tokio::task::spawn_blocking(move || {
        engine
            .get_changesets_by_date_range(&start_str, &end_str, &filters)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(changesets) => format_response(&changesets, format_opt.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

pub async fn get_changesets_by_id_range(
    State(state): State<AppState>,
    Path((start_raw, end_raw)): Path<(String, String)>,
    Query(mut params): Query<QueryParams>,
) -> Response {
    // Extract format extension from end parameter
    let (end_str, ext_format) = extract_extension(&end_raw);
    if params.format.is_none() {
        params.format = ext_format;
    }

    // Validate start_id
    let start_id = match start_raw.parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => {
            return ApiError::bad_request_with_details(
                "Invalid start ID",
                format!("'{}' is not a valid changeset ID. Must be a positive integer", start_raw)
            ).into_response()
        }
    };

    // Validate end_id
    let end_id = match end_str.parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => {
            return ApiError::bad_request_with_details(
                "Invalid end ID",
                format!("'{}' is not a valid changeset ID. Must be a positive integer", end_str)
            ).into_response()
        }
    };

    // Validate range
    if start_id > end_id {
        return ApiError::bad_request_with_details(
            "Invalid ID range",
            format!("start_id ({}) must be less than or equal to end_id ({})", start_id, end_id)
        ).into_response()
    }

    // Safety limit: Maximum 1,000,000 ID range
    const MAX_ID_RANGE: i64 = 1_000_000;
    let range_size = end_id - start_id + 1;
    if range_size > MAX_ID_RANGE {
        return ApiError::bad_request_with_details(
            "ID range too large",
            format!(
                "Requested range of {} IDs, but maximum allowed is {}. Please use a smaller range or pagination.",
                range_size, MAX_ID_RANGE
            ),
        ).into_response();
    }

    // Non-streaming mode - offload to blocking thread pool
    let engine = state.engine.clone();
    let filters = params.to_filters();
    let format_opt = params.format.clone();

    let result = tokio::task::spawn_blocking(move || {
        engine
            .get_changesets_by_id_range(start_id, end_id, &filters)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!("Task join error: {}", e);
        Err("Task panicked".to_string())
    });

    match result {
        Ok(changesets) => {
            // Safety limit: Check result set size
            const MAX_RESULT_SIZE: usize = 100_000;
            if changesets.len() > MAX_RESULT_SIZE {
                return ApiError::bad_request_with_details(
                    "Result set too large",
                    format!(
                        "Query returned {} changesets, but maximum allowed is {}. Please use a smaller ID range or pagination (limit/offset).",
                        changesets.len(), MAX_RESULT_SIZE
                    ),
                ).into_response();
            }
            format_response(&changesets, format_opt.as_deref(), false)
        }
        Err(e) => {
            tracing::error!("Query error: {}", e);
            ApiError::internal_error("Database query failed").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthConfig, AuthState};
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use duckdb::Connection;
    use serde_json::Value;
    use std::fs;
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
                replication_sequence BIGINT,
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
                (1, 100, TIMESTAMP '2024-05-01 00:00:00', TIMESTAMP '2024-05-01 01:00:00', false, 'alice', 10, 1.0, 2.0, 3.0, 4.0, 5, 6, 'first', MAP {'comment': 'first'}),
                (2, 110, TIMESTAMP '2024-05-02 12:00:00', NULL, true, 'bob', 20, 5.0, 6.0, 7.0, 8.0, 9, 0, 'second', MAP {'comment': 'second'});",
        )
        .expect("insert rows");

        let parquet_path_sql = parquet_path.to_string_lossy().replace('\'', "''");
        let copy_sql = format!(
            "COPY changesets TO '{}' (FORMAT PARQUET);",
            parquet_path_sql
        );
        conn.execute_batch(&copy_sql).expect("copy parquet");

        let engine = crate::query::QueryEngine::new(dir.path().to_str().unwrap()).expect("engine");
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
    async fn test_get_changesets_by_day_json() {
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_hour_json() {
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_minute_json() {
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_date_range() {
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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

    #[tokio::test]
    async fn test_get_changesets_by_id_range_json() {
        let (_dir, state) = setup_state().await;
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
                stream: None,
                order_by: None,
                order_direction: None,
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
    async fn test_get_changesets_by_id_range_invalid_start() {
        let (_dir, state) = setup_state().await;
        let response = get_changesets_by_id_range(
            State(state),
            Path(("invalid".to_string(), "100".to_string())),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_id_range_inverted() {
        let (_dir, state) = setup_state().await;
        let response = get_changesets_by_id_range(
            State(state),
            Path(("100".to_string(), "50".to_string())),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_changesets_by_id_range_json_suffix() {
        let (_dir, state) = setup_state().await;
        let response = get_changesets_by_id_range(
            State(state),
            Path(("1".to_string(), "2.json".to_string())),
            Query(QueryParams {
                format: None,
                user: None,
                uid: None,
                open: None,
                limit: None,
                offset: None,
                stream: None,
                order_by: None,
                order_direction: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok());
        assert_eq!(content_type, Some("application/json"));
    }

    #[tokio::test]
    async fn test_get_changesets_by_id_range_with_filter() {
        let (_dir, state) = setup_state().await;
        let response = get_changesets_by_id_range(
            State(state),
            Path(("1".to_string(), "2".to_string())),
            Query(QueryParams {
                format: Some("json".to_string()),
                user: Some("alice".to_string()),
                uid: None,
                open: None,
                limit: None,
                offset: None,
                stream: None,
                order_by: None,
                order_direction: None,
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
        assert_eq!(changesets[0]["user"], "alice");
    }
}
