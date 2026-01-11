//! Metadata models for dataset information
//!
//! Contains structures for dataset metadata, partition information, and statistics.

use serde::{Deserialize, Serialize};

/// Dataset-wide metadata
///
/// # TODO
/// - Add caching strategy
/// - Implement loading from partition files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub last_updated: String,
    pub latest_changeset_id: i64,
    pub earliest_changeset_id: i64,
    pub total_changesets: u64,
    pub data_through: String,
    pub update_frequency: String,
}

/// Information about a single partition
///
/// # TODO
/// - Calculate partition statistics
/// - Cache partition metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionMetadata {
    pub year: u32,
    pub month: u32,
    pub changeset_count: u64,
    pub file_size_bytes: u64,
    pub min_id: i64,
    pub max_id: i64,
    pub min_timestamp: i64,
    pub max_timestamp: i64,
}

/// Temporal coverage information
///
/// # TODO
/// - Implement coverage calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalCoverage {
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub start_changeset_id: i64,
    pub end_changeset_id: i64,
}

/// User statistics from changesets
///
/// # TODO
/// - Build user index from changesets
/// - Cache frequently accessed user stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStatistics {
    pub uid: i64,
    pub username: String,
    pub changeset_count: u64,
    pub first_changeset_id: i64,
    pub last_changeset_id: i64,
}
