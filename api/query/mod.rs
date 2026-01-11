//! Query execution module
//!
//! Contains the DuckDB query engine and SQL query building utilities.

pub mod engine;
pub mod builder;
pub mod partition;

pub use engine::QueryEngine;
pub use builder::QueryBuilder;
pub use partition::PartitionResolver;
