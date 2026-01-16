use crate::query::engine::QueryEngine;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct LatestChangesetCache {
    pub latest_changeset_id: i64,
    pub created_at: String,
    pub closed_at: String,
    pub open: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
    pub started_at: Instant,
    pub latest_changeset_cache: Arc<RwLock<Option<LatestChangesetCache>>>,
}
