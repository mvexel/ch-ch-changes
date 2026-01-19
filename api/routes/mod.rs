//! API route handlers
//!
//! This module contains all route handlers for the changeset historic API.
//! Routes are organized into submodules for batch/range queries and metadata.

pub mod changesets;
pub mod meta;

use crate::auth;
use crate::AppState;
use axum::{middleware, routing::get, Router};

/// Create the main API router with all routes
pub fn create_router(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/health", get(meta::health))
        .route("/auth/osm/start", get(auth::routes::oauth_start))
        .route("/auth/osm/callback", get(auth::routes::oauth_callback));

    let protected_routes = Router::new()
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
            "/changesets/date/from/:start/to/:end",
            get(changesets::get_changesets_by_date_range),
        )
        .route("/meta/replication", get(meta::get_replication_state))
        .route("/auth/me", get(auth::routes::get_me))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::auth_middleware,
        ));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
}
