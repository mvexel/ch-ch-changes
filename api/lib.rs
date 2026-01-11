//! Changeset Historic API Library
//!
//! Core modules for the changeset historic API server.

pub mod models;
pub mod routes;
pub mod query;
pub mod format;

use std::sync::Arc;
pub use models::{Changeset, Metadata};
pub use routes::create_router;
pub use query::engine::QueryEngine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
}
