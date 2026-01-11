//! Single changeset route handlers
//!
//! Handles requests for individual changesets by ID.

use axum::{
    extract::{Path, State, Query},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use crate::state::AppState;
use crate::format::{
    xml::XmlFormatter,
    json::JsonFormatter,
    Formatter,
};

#[derive(Deserialize)]
pub struct FormatQuery {
    format: Option<String>,
}

/// Get a single changeset by ID
pub async fn get_changeset(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<FormatQuery>,
) -> Response {
    let format = params.format.as_deref().unwrap_or("xml");
    match format {
        "json" => get_changeset_json(State(state), Path(id)).await.into_response(),
        _ => get_changeset_xml(State(state), Path(id)).await.into_response(),
    }
}

/// Get a single changeset in XML format
pub async fn get_changeset_xml(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.engine.get_changeset(id) {
        Ok(Some(changeset)) => {
            let formatter = XmlFormatter::new();
            match formatter.format_changeset(&changeset) {
                Ok(xml) => ([(header::CONTENT_TYPE, "application/xml")], xml).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Get a single changeset in JSON format
pub async fn get_changeset_json(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    match state.engine.get_changeset(id) {
        Ok(Some(changeset)) => {
            let formatter = JsonFormatter::new(false);
            match formatter.format_changeset(&changeset) {
                Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
                Err(e) => {
                    tracing::error!("Formatting error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Query error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
