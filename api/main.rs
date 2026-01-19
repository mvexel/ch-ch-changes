//! Changeset Historic API Server
//!
//! A high-performance HTTP API for querying historic OSM changeset data from Parquet files.

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;

mod auth;
mod constants;
mod format;
mod models;
mod query;
mod routes;
mod state;

// Re-export constants at crate root for use by submodules
pub use constants::{generator_string, API_VERSION};

use crate::query::engine::QueryEngine;
use crate::routes::create_router;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Load configuration
    let data_path = env::var("DATA_PATH").unwrap_or_else(|_| "./data".to_string());
    tracing::info!("Loading data from: {}", data_path);

    // Create query engine
    let engine = QueryEngine::new(&data_path).map_err(|e| {
        tracing::error!("Failed to initialize query engine: {}", e);
        e
    })?;

    let auth_config = auth::AuthConfig::from_env().map_err(|e| {
        tracing::error!("Auth configuration error: {}", e);
        e
    })?;
    let auth_state = auth::AuthState::new(auth_config).await.map_err(|e| {
        tracing::error!("Failed to initialize auth state: {}", e);
        e
    })?;
    tracing::info!("Auth DB: {}", auth_state.config.db_path.display());

    let state = AppState {
        engine: Arc::new(engine),
        started_at: Instant::now(),
        auth: auth_state,
    };

    // Create router
    let app = create_router(state).layer(CorsLayer::permissive());

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
