use crate::auth::AuthState;
use crate::query::engine::QueryEngine;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
    pub started_at: Instant,
    pub auth: AuthState,
}
