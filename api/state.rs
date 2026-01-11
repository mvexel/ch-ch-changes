use std::sync::Arc;
use std::time::Instant;
use crate::query::engine::QueryEngine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
    pub started_at: Instant,
}
