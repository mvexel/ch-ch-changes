//! One-time script to backfill metadata.json from existing parquet files
//!
//! Usage:
//!   cargo run --bin backfill-metadata -- --data-path /path/to/data
//!
//! This will scan all existing parquet files and generate metadata.json
//! with accurate statistics without doing full table scans.

use anyhow::{Context, Result};
use ch_ch_changes_core::metadata::{DatasetMetadata, PartitionMetadata};
use clap::Parser;
use duckdb::Connection;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Root data path (contains base/ and/or delta/ directories)
    #[arg(long, env = "DATA_PATH")]
    data_path: String,

    /// Output path for metadata.json (default: DATA_PATH/delta/metadata.json)
    #[arg(long)]
    output: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data_path = PathBuf::from(&args.data_path);

    println!("Scanning parquet files in {:?}...", data_path);

    // Collect all parquet files
    let mut files = Vec::new();

    // Check base directory
    let base_dir = data_path.join("base");
    if base_dir.exists() {
        collect_parquet_files(&base_dir, &mut files)?;
        println!("Found {} parquet files in base/", files.len());
    }

    // Check delta directory
    let delta_dir = data_path.join("delta");
    if delta_dir.exists() {
        let delta_start = files.len();
        collect_parquet_files(&delta_dir, &mut files)?;
        println!(
            "Found {} parquet files in delta/",
            files.len() - delta_start
        );
    }

    // Check root directory (flat structure)
    if files.is_empty() && data_path.exists() {
        collect_parquet_files(&data_path, &mut files)?;
        println!("Found {} parquet files in root", files.len());
    }

    if files.is_empty() {
        println!("No parquet files found. Nothing to do.");
        return Ok(());
    }

    println!(
        "\nComputing statistics from {} parquet files...",
        files.len()
    );
    println!("This may take a few minutes depending on dataset size...\n");

    // Compute statistics per partition
    let partition_stats = compute_partition_stats(&files)?;

    // Build metadata
    let mut partitions: Vec<PartitionMetadata> = partition_stats
        .into_iter()
        .map(|((year, month), stats)| PartitionMetadata {
            year,
            month,
            path: format!("year={}/month={:02}", year, month),
            count: stats.count,
            min_id: stats.min_id,
            max_id: stats.max_id,
            min_created_at: stats.min_created_at,
            max_created_at: stats.max_created_at,
        })
        .collect();

    partitions.sort_by_key(|p| (p.year, p.month));

    let total_changesets: u64 = partitions.iter().map(|p| p.count).sum();

    let metadata = DatasetMetadata {
        version: "1.0".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        total_changesets,
        partition_count: partitions.len(),
        min_created_at: partitions.iter().filter_map(|p| p.min_created_at).min(),
        max_created_at: partitions.iter().filter_map(|p| p.max_created_at).max(),
        partitions,
    };

    // Determine output path
    let output_path = if let Some(output) = args.output {
        PathBuf::from(output)
    } else if delta_dir.exists() {
        delta_dir.join("metadata.json")
    } else {
        data_path.join("metadata.json")
    };

    // Write metadata
    write_metadata(&output_path, &metadata)?;

    println!("✓ Successfully generated metadata.json");
    println!("  Location: {}", output_path.display());
    println!("  Total changesets: {}", metadata.total_changesets);
    println!("  Partitions: {}", metadata.partition_count);
    println!(
        "  Min changeset ID: {}",
        metadata
            .partitions
            .iter()
            .map(|p| p.min_id)
            .min()
            .unwrap_or(0)
    );
    println!(
        "  Max changeset ID: {}",
        metadata
            .partitions
            .iter()
            .map(|p| p.max_id)
            .max()
            .unwrap_or(0)
    );

    Ok(())
}

#[derive(Debug)]
struct PartitionStats {
    count: u64,
    min_id: i64,
    max_id: i64,
    min_created_at: Option<i64>,
    max_created_at: Option<i64>,
}

fn compute_partition_stats(files: &[PathBuf]) -> Result<HashMap<(u32, u32), PartitionStats>> {
    let conn = Connection::open_in_memory()?;
    let mut partition_stats: HashMap<(u32, u32), PartitionStats> = HashMap::new();

    // Add timestamp converter helper
    conn.execute_batch(
        "
        CREATE MACRO timestamp_ms(t) AS 
        CASE 
            WHEN t IS NULL THEN NULL 
            ELSE epoch(t) * 1000 + (item(date_part('microsecond', t), 0) / 1000)::BIGINT
        END;
    ",
    )?;

    for (idx, file) in files.iter().enumerate() {
        if (idx + 1) % 50 == 0 {
            println!("  Processed {}/{} files...", idx + 1, files.len());
        }

        // Extract year/month from path
        let Some((year, month)) = extract_year_month(file) else {
            eprintln!(
                "Warning: Could not parse year/month from {:?}, skipping",
                file
            );
            continue;
        };

        let key = (year, month);

        // Use DuckDB to efficiently read min/max/count from parquet file
        let file_sql = escape_sql_string(file);

        // Get count, min, and max in a single query
        let query = format!(
            "SELECT 
                COUNT(*) as cnt, 
                MIN(id) as min_id, 
                MAX(id) as max_id,
                MIN(created_at) as min_created,
                MAX(created_at) as max_created
             FROM read_parquet('{}')",
            file_sql
        );

        let mut stmt = conn
            .prepare(&query)
            .with_context(|| format!("Failed to prepare query for {:?}", file))?;

        let mut rows = stmt
            .query([])
            .with_context(|| format!("Failed to query {:?}", file))?;

        if let Some(row) = rows.next()? {
            let count: i64 = row.get("cnt")?;
            let min_id: Option<i64> = row.get("min_id")?;
            let max_id: Option<i64> = row.get("max_id")?;

            // DuckDB TIMESTAMP to millis is tricky from Rust via duckdb-rs if we don't custom parse
            // But let's try getting as Value or using the macro if needed.
            // Actually, simplified approach: get valid min/max timestamps as i64 via epoch_ms
            // duckdb-rs might map TIMESTAMP to string or chrono
            // Let's rely on retrieving as Option<chrono::NaiveDateTime> or similar and converting?
            // Or cast in SQL to epoch millis

            // Re-doing query to cast to epoch millis in SQL to be safe and easy
        }
    }

    // Redo loop with SQL cast
    for (idx, file) in files.iter().enumerate() {
        if (idx + 1) % 50 == 0 {
            println!("  Processed {}/{} files...", idx + 1, files.len());
        }

        // Extract year/month from path
        let Some((year, month)) = extract_year_month(file) else {
            eprintln!(
                "Warning: Could not parse year/month from {:?}, skipping",
                file
            );
            continue;
        };

        let key = (year, month);
        let file_sql = escape_sql_string(file);

        // We use epoch_ms(created_at) which DuckDB supports (since 0.8+ I think, or use date_part)
        // epoch(t)*1000 is easiest
        let query = format!(
            "SELECT 
                COUNT(*) as cnt, 
                MIN(id) as min_id, 
                MAX(id) as max_id,
                MIN(epoch(created_at)*1000) as min_created_ms,
                MAX(epoch(created_at)*1000) as max_created_ms
             FROM read_parquet('{}')",
            file_sql
        );

        let mut stmt = conn
            .prepare(&query)
            .with_context(|| format!("Failed to prepare query for {:?}", file))?;

        let mut rows = stmt
            .query([])
            .with_context(|| format!("Failed to query {:?}", file))?;

        if let Some(row) = rows.next()? {
            let count: i64 = row.get("cnt")?;
            let min_id: Option<i64> = row.get("min_id")?;
            let max_id: Option<i64> = row.get("max_id")?;
            let min_created_ms: Option<i64> = row.get("min_created_ms").ok().flatten(); // flatten because get returns Result<Option<T>>? No, get returns Result<T>.
                                                                                        // If column is NULL, get returns Ok(None) if T is Option.
                                                                                        // But sometimes SQL cast fails?
            let min_created_ms: Option<i64> = row.get("min_created_ms")?;
            let max_created_ms: Option<i64> = row.get("max_created_ms")?;

            partition_stats
                .entry(key)
                .and_modify(|stats| {
                    stats.count += count as u64;
                    if let Some(min) = min_id {
                        stats.min_id = stats.min_id.min(min);
                    }
                    if let Some(max) = max_id {
                        stats.max_id = stats.max_id.max(max);
                    }
                    if let Some(min) = min_created_ms {
                        stats.min_created_at =
                            Some(stats.min_created_at.map_or(min, |c| c.min(min)));
                    }
                    if let Some(max) = max_created_ms {
                        stats.max_created_at =
                            Some(stats.max_created_at.map_or(max, |c| c.max(max)));
                    }
                })
                .or_insert_with(|| PartitionStats {
                    count: count as u64,
                    min_id: min_id.unwrap_or(i64::MAX),
                    max_id: max_id.unwrap_or(i64::MIN),
                    min_created_at: min_created_ms,
                    max_created_at: max_created_ms,
                });
        }
    }

    Ok(partition_stats)
}

fn collect_parquet_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read directory {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip indexes directory
            if path.file_name().and_then(|name| name.to_str()) == Some("indexes") {
                continue;
            }
            collect_parquet_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            out.push(path);
        }
    }
    Ok(())
}

fn extract_year_month(path: &Path) -> Option<(u32, u32)> {
    let mut year: Option<u32> = None;
    let mut month: Option<u32> = None;

    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if let Some(value) = part.strip_prefix("year=") {
            year = value.parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("month=") {
            let trimmed = value.trim_end_matches(".parquet");
            month = trimmed.parse::<u32>().ok();
        }
    }

    match (year, month) {
        (Some(y), Some(m)) => Some((y, m)),
        _ => None,
    }
}

fn escape_sql_string(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn write_metadata(path: &Path, metadata: &DatasetMetadata) -> Result<()> {
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
