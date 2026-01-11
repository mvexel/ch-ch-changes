/// Example demonstrating the QueryEngine API
///
/// This example shows various ways to query changeset data using the QueryEngine.
///
/// To run this example:
/// ```bash
/// cargo run --example query_engine_demo -- --data-dir /path/to/parquet/data
/// ```

use anyhow::{Context, Result};
use changesets_to_parquet::api::{ChangesetFilter, QueryEngine, PartitionResolver};
use chrono::Utc;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Path to the parquet data directory
    #[arg(long)]
    data_dir: PathBuf,

    /// Optional: specific username to query
    #[arg(long)]
    username: Option<String>,

    /// Optional: specific changeset ID to query
    #[arg(long)]
    changeset_id: Option<i64>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("=== QueryEngine Demo ===\n");

    // Create the query engine
    println!("Initializing QueryEngine with data directory: {:?}", args.data_dir);
    let engine = QueryEngine::new(&args.data_dir)
        .context("Failed to create QueryEngine")?;

    println!("✓ QueryEngine initialized\n");

    // Example 1: List available partitions
    println!("--- Example 1: List Available Partitions ---");
    list_partitions(&args.data_dir)?;

    // Example 2: Count total changesets
    println!("\n--- Example 2: Count Total Changesets ---");
    let total = engine.count_changesets()
        .context("Failed to count changesets")?;
    println!("Total changesets in dataset: {}", total);

    // Example 3: Query by specific ID if provided
    if let Some(id) = args.changeset_id {
        println!("\n--- Example 3: Query Specific Changeset by ID ---");
        query_by_id(&engine, id)?;
    }

    // Example 4: Query by username if provided
    if let Some(username) = args.username {
        println!("\n--- Example 4: Query Changesets by Username ---");
        query_by_username(&engine, &username)?;
    }

    // Example 5: Date range query (last month if we can determine it)
    println!("\n--- Example 5: Date Range Query ---");
    query_date_range(&engine)?;

    // Example 6: Query open changesets
    println!("\n--- Example 6: Open Changesets ---");
    query_open_changesets(&engine)?;

    // Example 7: Advanced filtering
    println!("\n--- Example 7: Advanced Filtering ---");
    query_with_filters(&engine)?;

    println!("\n✓ Demo completed successfully");
    Ok(())
}

fn list_partitions(data_dir: &PathBuf) -> Result<()> {
    let partitions = PartitionResolver::list_partitions(data_dir)
        .context("Failed to list partitions")?;

    if partitions.is_empty() {
        println!("No partitions found in the data directory");
        return Ok(());
    }

    println!("Found {} partitions:", partitions.len());
    for (idx, partition) in partitions.iter().enumerate() {
        if idx < 5 {
            println!("  {}", partition.path());
        } else if idx == 5 {
            println!("  ... and {} more", partitions.len() - 5);
            break;
        }
    }

    Ok(())
}

fn query_by_id(engine: &QueryEngine, id: i64) -> Result<()> {
    match engine.get_changeset_by_id(id)
        .context("Failed to query changeset by ID")?
    {
        Some(changeset) => {
            println!("Found changeset {}:", id);
            println!("  User: {:?}", changeset.user);
            println!("  Created: {:?}", changeset.created_at);
            println!("  Closed: {:?}", changeset.closed_at);
            println!("  Open: {}", changeset.open);
            println!("  Changes: {}", changeset.num_changes);
            println!("  Comments: {}", changeset.comments_count);
            if let Some(desc) = &changeset.description {
                println!("  Description: {}", desc);
            }
        }
        None => {
            println!("Changeset {} not found", id);
        }
    }
    Ok(())
}

fn query_by_username(engine: &QueryEngine, username: &str) -> Result<()> {
    let changesets = engine.get_changesets_by_user(username)
        .context("Failed to query changesets by user")?;

    println!("Found {} changesets by {}", changesets.len(), username);

    // Show summary
    let closed_count = changesets.iter().filter(|cs| !cs.open).count();
    let open_count = changesets.iter().filter(|cs| cs.open).count();
    let total_changes: u32 = changesets.iter().map(|cs| cs.num_changes).sum();

    println!("  Closed: {}", closed_count);
    println!("  Open: {}", open_count);
    println!("  Total changes: {}", total_changes);

    // Show first 3 changesets
    println!("\n  First 3 changesets:");
    for changeset in changesets.iter().take(3) {
        println!("    ID: {}, created: {:?}, changes: {}",
                 changeset.id, changeset.created_at, changeset.num_changes);
    }

    Ok(())
}

fn query_date_range(engine: &QueryEngine) -> Result<()> {
    // Query the last 30 days (from today)
    let now = Utc::now();
    let thirty_days_ago = now - chrono::Duration::days(30);

    println!("Querying changesets from {} to {}",
             thirty_days_ago.date_naive(), now.date_naive());

    let filter = ChangesetFilter {
        limit: Some(10),
        ..Default::default()
    };

    let changesets = engine.get_changesets_by_date_range_with_filter(
        &thirty_days_ago,
        &now,
        filter,
    ).context("Failed to query date range")?;

    println!("Found {} changesets in the last 30 days (showing first 10)",
             changesets.len());

    for changeset in &changesets {
        println!("  ID: {}, user: {:?}, changes: {}",
                 changeset.id, changeset.user, changeset.num_changes);
    }

    Ok(())
}

fn query_open_changesets(engine: &QueryEngine) -> Result<()> {
    let changesets = engine.get_open_changesets(10)
        .context("Failed to query open changesets")?;

    println!("Found {} open changesets (showing first 10)", changesets.len());

    for changeset in &changesets {
        println!("  ID: {}, user: {:?}, created: {:?}",
                 changeset.id, changeset.user, changeset.created_at);
    }

    Ok(())
}

fn query_with_filters(engine: &QueryEngine) -> Result<()> {
    // Example: Find recent closed changesets with many changes
    let now = Utc::now();
    let seven_days_ago = now - chrono::Duration::days(7);

    let filter = ChangesetFilter {
        open: Some(false),  // Only closed changesets
        limit: Some(5),
        ..Default::default()
    };

    let changesets = engine.get_changesets_by_date_range_with_filter(
        &seven_days_ago,
        &now,
        filter,
    ).context("Failed to query with filters")?;

    println!("Found {} closed changesets in the last 7 days (showing first 5):",
             changesets.len());

    // Sort by number of changes (descending)
    let mut sorted = changesets;
    sorted.sort_by(|a, b| b.num_changes.cmp(&a.num_changes));

    for changeset in sorted.iter().take(5) {
        println!("  ID: {}, user: {:?}, changes: {}, comments: {}",
                 changeset.id, changeset.user, changeset.num_changes,
                 changeset.comments_count);
    }

    Ok(())
}
