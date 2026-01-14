//! Changeset Historic API Library
//!
//! Core modules for the changeset historic API server.

pub mod format;
pub mod models;
pub mod query;
pub mod routes;

pub use models::{Changeset, Metadata};
pub use query::engine::QueryEngine;
pub use routes::create_router;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
}
