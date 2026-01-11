//! API route handlers
//!
//! This module contains all route handlers for the changeset historic API.
//! Routes are organized into submodules for single changesets, batch/range queries, and metadata.

pub mod changeset;
pub mod changesets;
pub mod meta;

use axum::{Router, routing::get};
use crate::AppState;

/// Create the main API router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/changeset/:id", get(changeset::get_changeset))
        .route("/changeset/:id.xml", get(changeset::get_changeset_xml))
        .route("/changeset/:id.json", get(changeset::get_changeset_json))
        .route("/changesets/day/:date", get(changesets::get_changesets_by_day))
        .route("/changesets/from/:start/to/:end", get(changesets::get_changesets_range))
        .with_state(state)
}
