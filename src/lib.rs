use std::collections::HashMap;

pub mod builder;
pub mod indexing;
pub mod models;
pub mod parsing;
pub mod schema;
pub mod writer;

/// Represents a changeset in the OSM data
#[derive(Debug, Default, Clone)]
pub struct Changeset {
    pub id: i64,
    pub created_at: Option<i64>,
    pub closed_at: Option<i64>,
    pub open: bool,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub min_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lat: Option<f64>,
    pub max_lon: Option<f64>,
    pub num_changes: u32,
    pub comments_count: u32,
    pub description: Option<String>,
    pub tags: HashMap<String, String>,
}