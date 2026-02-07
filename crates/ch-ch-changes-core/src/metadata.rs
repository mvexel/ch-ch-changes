//! Incremental metadata tracking for dataset statistics
//!
//! Maintains a metadata.json file with dataset statistics that can be updated
//! incrementally as new batches are processed, avoiding expensive full table scans.

use anyhow::{Context, Result};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Dataset-level metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadata {
    pub version: String,
    pub generated_at: String,
    pub total_changesets: u64,
    pub partition_count: usize,
    pub min_created_at: Option<i64>,
    pub max_created_at: Option<i64>,
    pub partitions: Vec<PartitionMetadata>,
}

/// Per-partition metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionMetadata {
    pub year: u32,
    pub month: u32,
    pub path: String,
    pub count: u64,
    pub min_id: i64,
    pub max_id: i64,
    pub min_created_at: Option<i64>,
    pub max_created_at: Option<i64>,
}

/// Statistics from a batch of changesets
#[derive(Debug, Clone)]
pub struct BatchStats {
    pub partition_counts: HashMap<(u32, u32), u64>,
    pub partition_min_ids: HashMap<(u32, u32), i64>,
    pub partition_max_ids: HashMap<(u32, u32), i64>,
    pub partition_min_created_at: HashMap<(u32, u32), i64>,
    pub partition_max_created_at: HashMap<(u32, u32), i64>,
    pub total_count: u64,
}

impl BatchStats {
    /// Create empty batch stats
    pub fn new() -> Self {
        Self {
            partition_counts: HashMap::new(),
            partition_min_ids: HashMap::new(),
            partition_max_ids: HashMap::new(),
            partition_min_created_at: HashMap::new(),
            partition_max_created_at: HashMap::new(),
            total_count: 0,
        }
    }

    /// Compute stats from a collection of changesets
    pub fn from_changesets(changesets: &[crate::Changeset]) -> Self {
        let mut stats = Self::new();

        for cs in changesets {
            // Extract year/month from created_at timestamp (millis since epoch)
            if let Some(created_at_ms) = cs.created_at {
                let created_at_secs = created_at_ms / 1000;
                let dt = chrono::DateTime::from_timestamp(created_at_secs, 0)
                    .unwrap_or_else(|| chrono::Utc::now());

                let year = dt.year() as u32;
                let month = dt.month();
                let key = (year, month);

                // Update count
                *stats.partition_counts.entry(key).or_insert(0) += 1;

                // Update min_id
                stats
                    .partition_min_ids
                    .entry(key)
                    .and_modify(|min| *min = (*min).min(cs.id))
                    .or_insert(cs.id);

                // Update max_id
                stats
                    .partition_max_ids
                    .entry(key)
                    .and_modify(|max| *max = (*max).max(cs.id))
                    .or_insert(cs.id);

                // Update min_created_at
                stats
                    .partition_min_created_at
                    .entry(key)
                    .and_modify(|min| *min = (*min).min(created_at_ms))
                    .or_insert(created_at_ms);

                // Update max_created_at
                stats
                    .partition_max_created_at
                    .entry(key)
                    .and_modify(|max| *max = (*max).max(created_at_ms))
                    .or_insert(created_at_ms);
            }
        }

        stats.total_count = changesets.len() as u64;
        stats
    }
}

impl Default for BatchStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Load metadata from file, returning None if file doesn't exist
pub fn load_metadata(metadata_path: &Path) -> Result<Option<DatasetMetadata>> {
    if !metadata_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(metadata_path)
        .with_context(|| format!("Failed to read metadata from {:?}", metadata_path))?;

    let metadata: DatasetMetadata = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse metadata from {:?}", metadata_path))?;

    Ok(Some(metadata))
}

/// Update metadata incrementally with new batch statistics
pub fn update_metadata_incremental(metadata_path: &Path, batch_stats: &BatchStats) -> Result<()> {
    // Load existing metadata or create new
    let mut metadata = load_metadata(metadata_path)?.unwrap_or_else(|| DatasetMetadata {
        version: "1.0".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        total_changesets: 0,
        partition_count: 0,
        min_created_at: None,
        max_created_at: None,
        partitions: Vec::new(),
    });

    // Update partition metadata
    let mut partition_map: HashMap<(u32, u32), PartitionMetadata> = metadata
        .partitions
        .into_iter()
        .map(|p| ((p.year, p.month), p))
        .collect();

    for ((year, month), count) in &batch_stats.partition_counts {
        let key = (*year, *month);
        let min_id = batch_stats
            .partition_min_ids
            .get(&key)
            .copied()
            .unwrap_or(i64::MAX);
        let max_id = batch_stats
            .partition_max_ids
            .get(&key)
            .copied()
            .unwrap_or(i64::MIN);
        let min_created_at = batch_stats.partition_min_created_at.get(&key).copied();
        let max_created_at = batch_stats.partition_max_created_at.get(&key).copied();

        partition_map
            .entry(key)
            .and_modify(|p| {
                p.count += count;
                p.min_id = p.min_id.min(min_id);
                p.max_id = p.max_id.max(max_id);
                if let Some(new_min) = min_created_at {
                    p.min_created_at =
                        Some(p.min_created_at.map_or(new_min, |curr| curr.min(new_min)));
                }
                if let Some(new_max) = max_created_at {
                    p.max_created_at =
                        Some(p.max_created_at.map_or(new_max, |curr| curr.max(new_max)));
                }
            })
            .or_insert_with(|| PartitionMetadata {
                year: *year,
                month: *month,
                path: format!("year={}/month={:02}", year, month),
                count: *count,
                min_id,
                max_id,
                min_created_at,
                max_created_at,
            });
    }

    // Convert back to sorted list
    let mut partitions: Vec<_> = partition_map.into_values().collect();
    partitions.sort_by_key(|p| (p.year, p.month));

    // Update totals
    metadata.total_changesets += batch_stats.total_count;
    metadata.partition_count = partitions.len();

    // Recompute global min/max created_at from partitions
    metadata.min_created_at = partitions.iter().filter_map(|p| p.min_created_at).min();
    metadata.max_created_at = partitions.iter().filter_map(|p| p.max_created_at).max();

    metadata.partitions = partitions;
    metadata.generated_at = chrono::Utc::now().to_rfc3339();

    // Write atomically using a temp file
    write_metadata_atomic(metadata_path, &metadata)?;

    Ok(())
}

/// Write metadata atomically using a temporary file
fn write_metadata_atomic(path: &Path, metadata: &DatasetMetadata) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");

    let mut file = File::create(&tmp_path)
        .with_context(|| format!("Failed to create temp metadata file {:?}", tmp_path))?;

    let json = serde_json::to_string_pretty(metadata).context("Failed to serialize metadata")?;

    writeln!(file, "{}", json)
        .with_context(|| format!("Failed to write temp metadata file {:?}", tmp_path))?;

    file.sync_all()
        .with_context(|| format!("Failed to sync temp metadata file {:?}", tmp_path))?;

    fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to rename {:?} to {:?}", tmp_path, path))?;

    Ok(())
}

/// Find metadata.json file in standard locations (base/metadata.json or delta/metadata.json)
pub fn find_metadata_file(data_path: &Path) -> Option<PathBuf> {
    let candidates = [
        data_path.join("delta/metadata.json"),
        data_path.join("base/metadata.json"),
        data_path.join("metadata.json"),
    ];

    candidates.iter().find(|p| p.exists()).cloned()
}

/// Get the metadata path for a data directory (prefers delta/metadata.json)
pub fn get_metadata_path(data_path: &Path) -> PathBuf {
    let delta_dir = data_path.join("delta");
    if delta_dir.exists() {
        delta_dir.join("metadata.json")
    } else {
        data_path.join("metadata.json")
    }
}
