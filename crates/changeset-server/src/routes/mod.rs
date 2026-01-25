//! API route handlers
//!
//! This module contains all route handlers for the changeset historic API.
//! Routes are organized into submodules for batch/range queries and metadata.

pub mod changesets;
pub mod meta;

use crate::auth;
use crate::openapi::ApiDoc;
use crate::AppState;
use axum::{middleware, routing::get, Router};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Create the main API router with all routes
pub fn create_router(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/health", get(meta::health))
        .route("/meta/state", get(meta::get_replication_state))
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
        .route(
            "/changesets/id/from/:start/to/:end",
            get(changesets::get_changesets_by_id_range),
        )
        .route("/auth/me", get(auth::routes::get_me))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::auth_middleware,
        ));

    // OpenAPI and Swagger UI routes
    let swagger_ui = SwaggerUi::new("/swagger-ui")
        .url("/openapi.json", ApiDoc::openapi());

    Router::new()
        .merge(swagger_ui)
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
}
