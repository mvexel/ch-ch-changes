//! API route handlers
//!
//! This module contains all route handlers for the changeset historic API.
//! Routes are organized into submodules for single changesets, batch/range queries, and metadata.

pub mod changeset;
pub mod changesets;
pub mod meta;

use crate::AppState;
use axum::{routing::get, Router};

/// Create the main API router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/changeset/:id", get(changeset::get_changeset))
        .route(
            "/changesets/day/:date",
            get(changesets::get_changesets_by_day),
        )
        .route(
            "/changesets/day/:date/hour/:hour",
            get(changesets::get_changesets_by_hour),
        )
        .route(
            "/changesets/day/:date/hour/:hour/minute/:minute",
            get(changesets::get_changesets_by_minute),
        )
        .route(
            "/changesets/id/from/:start/to/:end",
            get(changesets::get_changesets_by_id_range),
        )
        .route(
            "/changesets/date/from/:start/to/:end",
            get(changesets::get_changesets_by_date_range),
        )
        .route("/health", get(meta::health))
        .route("/meta/tip", get(meta::get_tip))
        .route("/meta/coverage", get(meta::get_coverage))
        .with_state(state)
}
