//! Batch and range changeset route handlers

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use chrono::{NaiveDate, Utc, TimeZone};
use crate::state::AppState;
use crate::models::Changeset;
use crate::query::engine::QueryFilters;
use crate::format::{
    xml::XmlFormatter,
    json::JsonFormatter,
    Formatter,
};

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

fn format_response(changesets: &[Changeset], format: Option<&str>, pretty: bool) -> Response {
    match format {
        Some("json") => {
            let formatter = JsonFormatter::new(pretty);
            match formatter.format_changesets(changesets) {
                Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        _ => {
            let formatter = XmlFormatter::new();
            match formatter.format_changesets(changesets) {
                Ok(xml) => ([(header::CONTENT_TYPE, "application/xml")], xml).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
    }
}

pub async fn get_changesets_by_day(
    State(state): State<AppState>,
    Path(date_str): Path<String>,
    Query(params): Query<QueryParams>,
) -> Response {
    let date = match NaiveDate::parse_from_str(&date_str, "%Y%m%d") {
        Ok(d) => d,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let start = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59).unwrap());

    match state.engine.get_changesets_by_date_range(
        &start.to_rfc3339(), 
        &end.to_rfc3339(), 
        &params.to_filters()
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn handle_id_range(
    state: AppState,
    start_id: i64,
    end_id: i64,
    params: QueryParams,
) -> Response {
    match state.engine.get_changesets_by_id_range(
        start_id, 
        end_id, 
        &params.to_filters()
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn handle_date_range(
    state: AppState,
    start_str: String,
    end_str: String,
    params: QueryParams,
) -> Response {
    let start_date = match NaiveDate::parse_from_str(&start_str, "%Y%m%d") {
        Ok(d) => Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let end_date = match NaiveDate::parse_from_str(&end_str, "%Y%m%d") {
        Ok(d) => Utc.from_utc_datetime(&d.and_hms_opt(23, 59, 59).unwrap()),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match state.engine.get_changesets_by_date_range(
        &start_date.to_rfc3339(), 
        &end_date.to_rfc3339(), 
        &params.to_filters()
    ) {
        Ok(changesets) => format_response(&changesets, params.format.as_deref(), false),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn get_changesets_range(
    State(state): State<AppState>,
    Path((start, end)): Path<(String, String)>,
    Query(params): Query<QueryParams>,
) -> Response {
    // Try to parse as IDs first
    if let (Ok(start_id), Ok(end_id)) = (start.parse::<i64>(), end.parse::<i64>()) {
        return handle_id_range(state, start_id, end_id, params).await;
    }
    
    // Fallback to date range
    handle_date_range(state, start, end, params).await
}

pub async fn get_changesets_by_hour(
    State(_state): State<AppState>,
    Path((_date, _hour)): Path<(String, String)>,
) -> Response {
    // TODO: Implement
    StatusCode::NOT_IMPLEMENTED.into_response()
}

pub async fn get_changesets_by_minute(
    State(_state): State<AppState>,
    Path((_date, _hour, _minute)): Path<(String, String, String)>,
) -> Response {
    // TODO: Implement
    StatusCode::NOT_IMPLEMENTED.into_response()
}
