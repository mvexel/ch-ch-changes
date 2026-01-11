//! Changeset Historic API Server
//!
//! A high-performance HTTP API for querying historic OSM changeset data from Parquet files.

use std::net::SocketAddr;
use std::sync::Arc;
use std::env;
use tower_http::cors::CorsLayer;
use tracing_subscriber;

mod models;
mod routes;
mod query;
mod format;
mod state;

use crate::state::AppState;
use crate::query::engine::QueryEngine;
use crate::routes::create_router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    // Load configuration
    let data_path = env::var("DATA_PATH").unwrap_or_else(|_| "./data".to_string());
    tracing::info!("Loading data from: {}", data_path);

    // Create query engine
    let engine = QueryEngine::new(&data_path).map_err(|e| {
        tracing::error!("Failed to initialize query engine: {}", e);
        e
    })?;

    let state = AppState {
        engine: Arc::new(engine),
    };

    // Create router
    let app = create_router(state)
        .layer(CorsLayer::permissive());

    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
