//! Shared constants for the API server

/// API version - single source of truth (from Cargo.toml)
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Generator string for OSM-compatible output
pub fn generator_string() -> String {
    format!("changeset-historic-api/{}", API_VERSION)
}
