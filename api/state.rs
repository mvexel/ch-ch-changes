use std::sync::Arc;
use crate::query::engine::QueryEngine;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<QueryEngine>,
}
