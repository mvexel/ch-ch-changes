use crate::builder::BatchBuilders;
use crate::models::UserStats;
use crate::Changeset;
use anyhow::Result;
use arrow::array::{
    ArrayRef, Int64Builder, StringBuilder, TimestampMillisecondBuilder, UInt64Builder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use duckdb::{params, Connection};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

/// Batch size for flushing open changesets to disk (keeps memory bounded)
const INDEX_FLUSH_BATCH_SIZE: usize = 10_000;
/// Threshold for flushing user stats to disk (avoid unbounded memory)
const USER_STATS_FLUSH_THRESHOLD: usize = 100_000;

pub struct IndexCollector {
    user_stats: HashMap<i64, UserStats>,
    output_dir: PathBuf,
    // Streaming writer for open changesets (avoids unbounded memory growth)
    open_changesets_writer: ArrowWriter<File>,
    open_changesets_builders: BatchBuilders,
    open_changesets_count: usize,
    schema: Arc<Schema>,
    user_stats_part_count: usize,
}

impl IndexCollector {
    pub fn new(output_dir: &PathBuf, schema: Arc<Schema>) -> Result<Self> {
        let indexes_dir = output_dir.join("indexes");
        fs::create_dir_all(&indexes_dir)?;

        let file_path = indexes_dir.join("open_changesets.parquet");
        let file = File::create(&file_path)?;

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        let writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
        let builders = BatchBuilders::with_capacity(INDEX_FLUSH_BATCH_SIZE);

        Ok(Self {
            user_stats: HashMap::new(),
            output_dir: output_dir.clone(),
            open_changesets_writer: writer,
            open_changesets_builders: builders,
            open_changesets_count: 0,
            schema,
            user_stats_part_count: 0,
        })
    }

    pub fn record(&mut self, cs: &Changeset) -> Result<()> {
        // Track open changesets - stream to disk instead of accumulating in memory
        if cs.open {
            self.open_changesets_builders.append(cs);
            self.open_changesets_count += 1;

            // Flush to disk when batch is full
            if self.open_changesets_builders.len() >= INDEX_FLUSH_BATCH_SIZE {
                self.flush_open_changesets()?;
            }
        }

        // Track user stats
        if let Some(uid) = cs.uid {
            let stats = self.user_stats.entry(uid).or_insert_with(|| UserStats {
                uid,
                username: cs.user.clone(),
                ..Default::default()
            });
            stats.changeset_count += 1;

            // Update username if we have one (may change over time)
            if cs.user.is_some() {
                stats.username = cs.user.clone();
            }

            // Track first/last changeset times
            if let Some(ts) = cs.created_at {
                match stats.first_changeset_at {
                    None => stats.first_changeset_at = Some(ts),
                    Some(first) if ts < first => stats.first_changeset_at = Some(ts),
                    _ => {}
                }
                match stats.last_changeset_at {
                    None => stats.last_changeset_at = Some(ts),
                    Some(last) if ts > last => stats.last_changeset_at = Some(ts),
                    _ => {}
                }
            }
            
            // Check memory threshold for user stats
            if self.user_stats.len() >= USER_STATS_FLUSH_THRESHOLD {
                self.flush_user_stats_partial()?;
            }
        }
        Ok(())
    }

    fn flush_open_changesets(&mut self) -> Result<()> {
        if !self.open_changesets_builders.is_empty() {
            let batch = self.open_changesets_builders.finish_batch(&self.schema)?;
            self.open_changesets_writer.write(&batch)?;
            // Reset builders for next batch
            self.open_changesets_builders = BatchBuilders::with_capacity(INDEX_FLUSH_BATCH_SIZE);
        }
        Ok(())
    }
    
    fn flush_user_stats_partial(&mut self) -> Result<()> {
        if self.user_stats.is_empty() {
            return Ok(());
        }
        
        let indexes_dir = self.output_dir.join("indexes");
        fs::create_dir_all(&indexes_dir)?;
        
        let file_path = indexes_dir.join(format!("user_stats_part_{:05}.parquet", self.user_stats_part_count));
        self.user_stats_part_count += 1;
        
        self.write_user_stats_file(&file_path)?;
        
        // Clear memory
        self.user_stats.clear();
        
        Ok(())
    }

    fn write_user_stats_file(&self, file_path: &PathBuf) -> Result<()> {
        let file = File::create(file_path)?;

        let user_schema = Arc::new(Schema::new(vec![
            Field::new("uid", DataType::Int64, false),
            Field::new("username", DataType::Utf8, true),
            Field::new("changeset_count", DataType::UInt64, false),
            Field::new(
                "first_changeset_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ),
            Field::new(
                "last_changeset_at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ),
        ]));

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        let mut writer = ArrowWriter::try_new(file, user_schema.clone(), Some(props))?;

        // Build arrays
        let capacity = self.user_stats.len();
        let mut uid_builder = Int64Builder::with_capacity(capacity);
        let mut username_builder = StringBuilder::with_capacity(capacity, capacity * 16);
        let mut count_builder = UInt64Builder::with_capacity(capacity);
        let mut first_builder = TimestampMillisecondBuilder::with_capacity(capacity);
        let mut last_builder = TimestampMillisecondBuilder::with_capacity(capacity);

        for stats in self.user_stats.values() {
            uid_builder.append_value(stats.uid);
            match &stats.username {
                Some(name) => username_builder.append_value(name),
                None => username_builder.append_null(),
            }
            count_builder.append_value(stats.changeset_count);
            match stats.first_changeset_at {
                Some(ts) => first_builder.append_value(ts),
                None => first_builder.append_null(),
            }
            match stats.last_changeset_at {
                Some(ts) => last_builder.append_value(ts),
                None => last_builder.append_null(),
            }
        }

        let columns: Vec<ArrayRef> = vec![
            Arc::new(uid_builder.finish()),
            Arc::new(username_builder.finish()),
            Arc::new(count_builder.finish()),
            Arc::new(first_builder.finish()),
            Arc::new(last_builder.finish()),
        ];

        let batch = RecordBatch::try_new(user_schema, columns)?;
        writer.write(&batch)?;
        writer.close()?;
        
        Ok(())
    }

    pub fn write_indexes(mut self) -> Result<()> {
        // Flush any remaining open changesets
        self.flush_open_changesets()?;

        // Flush remaining user stats
        self.flush_user_stats_partial()?;

        // Capture needed variables before partial move of self
        let output_dir = self.output_dir.clone();
        let part_count = self.user_stats_part_count;

        // Close the open changesets writer (consumes the writer)
        self.open_changesets_writer.close()?;
        println!(
            "  Index open_changesets.parquet: {} changesets",
            self.open_changesets_count
        );

        // Merge user stats if we have multiple parts, or just rename if one, or just done.
        // We should merge to be clean.
        Self::merge_user_stats_static(&output_dir, part_count)?;

        Ok(())
    }
    
    fn merge_user_stats_static(output_dir: &PathBuf, part_count: usize) -> Result<()> {
        let indexes_dir = output_dir.join("indexes");
        let final_path = indexes_dir.join("user_stats.parquet");
        
        // If we have no parts, write empty?
        if part_count == 0 {
             // Just create empty file or skip?
             // Re-using write_user_stats_file with empty map would work but self is consumed/we are in &self
             // We can just create an empty file with the schema.
             // But simpler is to check if we have files.
             return Ok(());
        }
        
        println!("  Merging {} user stats partitions...", part_count);
        
        let conn = Connection::open_in_memory()?;
        
        // Use glob to read all parts
        let glob_pattern = indexes_dir.join("user_stats_part_*.parquet");
        let glob_str = glob_pattern.to_str().ok_or_else(|| anyhow::anyhow!("Invalid path"))?;
        
        // Query to merge
        // We take the username from ANY_VALUE (randomly picked, usually last one saw or first)
        // SUM changeset_count
        // MIN first_changeset_at
        // MAX last_changeset_at
        let query = format!(
            "COPY (
                SELECT 
                    uid, 
                    ANY_VALUE(username) as username, 
                    SUM(changeset_count) as changeset_count, 
                    MIN(first_changeset_at) as first_changeset_at, 
                    MAX(last_changeset_at) as last_changeset_at 
                FROM read_parquet('{}') 
                GROUP BY uid
            ) TO '{}' (FORMAT PARQUET, COMPRESSION 'SNAPPY')", 
            glob_str, 
            final_path.to_str().unwrap()
        );
        
        conn.execute(&query, params![])?;
        
        // Clean up partial files
        for i in 0..part_count {
            let part_path = indexes_dir.join(format!("user_stats_part_{:05}.parquet", i));
            if part_path.exists() {
                fs::remove_file(part_path)?;
            }
        }
        
        println!("  Index user_stats.parquet merged and ready.");
        
        Ok(())
    }
}

/// Write metadata.json file with partition information
pub fn write_metadata(
    output_dir: &PathBuf,
    total_count: usize,
    partition_counts: &HashMap<crate::models::PartitionKey, usize>,
) -> Result<()> {
    use std::io::Write;

    let metadata_path = output_dir.join("metadata.json");
    let mut file = File::create(&metadata_path)?;

    // Build partition list sorted by year/month
    let mut partitions: Vec<_> = partition_counts
        .iter()
        .map(|(k, v)| {
            serde_json::json!({
                "year": k.year,
                "month": k.month,
                "path": k.dir_path(),
                "count": v
            })
        })
        .collect();

    partitions.sort_by(|a, b| {
        let ya = a["year"].as_i64().unwrap();
        let yb = b["year"].as_i64().unwrap();
        let ma = a["month"].as_u64().unwrap();
        let mb = b["month"].as_u64().unwrap();
        (ya, ma).cmp(&(yb, mb))
    });

    let metadata = serde_json::json!({
        "version": "1.0",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "total_changesets": total_count,
        "partition_count": partition_counts.len(),
        "partitions": partitions
    });

    writeln!(file, "{}", serde_json::to_string_pretty(&metadata)?)?;

    println!("  Wrote metadata.json");

    Ok(())
}
