//! Metadata models for dataset information
//!
//! Contains structures for dataset metadata, partition information, and statistics.

use serde::{Deserialize, Serialize};

/// Dataset-wide metadata
///
/// Intended to be populated from cached partition/index data.
#[allow(dead_code)]
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
/// Intended to be populated from partition stats and cached.
#[allow(dead_code)]
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
/// Intended to be computed from partition stats.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalCoverage {
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub start_changeset_id: i64,
    pub end_changeset_id: i64,
}

/// User statistics from changesets
///
/// Intended to be populated from user index data.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStatistics {
    pub uid: i64,
    pub username: String,
    pub changeset_count: u64,
    pub first_changeset_id: i64,
    pub last_changeset_id: i64,
}
