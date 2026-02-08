use crate::builder::BatchBuilders;
use crate::models::{PartitionKey, PartitionStats};
use crate::Changeset;
use anyhow::Result;
use arrow::array::{ArrayRef, Int64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;

/// Batch size for flushing open changesets to disk (keeps memory bounded)
const INDEX_FLUSH_BATCH_SIZE: usize = 10_000;

pub struct IndexCollector {
    // Streaming writer for open changesets (avoids unbounded memory growth)
    open_changesets_writer: ArrowWriter<File>,
    open_changesets_builders: BatchBuilders,
    open_changesets_count: usize,
    schema: Arc<Schema>,
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
            open_changesets_writer: writer,
            open_changesets_builders: builders,
            open_changesets_count: 0,
            schema,
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

    pub fn write_indexes(mut self) -> Result<()> {
        // Flush any remaining open changesets
        self.flush_open_changesets()?;

        // Close the open changesets writer (consumes the writer)
        self.open_changesets_writer.close()?;
        println!(
            "  Index open_changesets.parquet: {} changesets",
            self.open_changesets_count
        );

        Ok(())
    }
}

pub fn write_partition_stats_index(
    output_dir: &PathBuf,
    stats: &HashMap<PartitionKey, PartitionStats>,
) -> Result<()> {
    if stats.is_empty() {
        return Ok(());
    }

    let indexes_dir = output_dir.join("indexes");
    fs::create_dir_all(&indexes_dir)?;

    let file_path = indexes_dir.join("partition_stats.parquet");
    let file = File::create(&file_path)?;

    let schema = Arc::new(Schema::new(vec![
        Field::new("path", DataType::Utf8, false),
        Field::new("year", DataType::Int64, false),
        Field::new("month", DataType::Int64, false),
        Field::new("changeset_count", DataType::Int64, false),
        Field::new("min_id", DataType::Int64, false),
        Field::new("max_id", DataType::Int64, false),
        Field::new("min_created_at", DataType::Int64, true),
        Field::new("max_created_at", DataType::Int64, true),
        Field::new("file_size_bytes", DataType::Int64, false),
    ]));

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;

    let mut entries: Vec<_> = stats.iter().collect();
    entries.sort_by_key(|(key, _)| (key.year, key.month));
    let entry_count = entries.len();

    let capacity = entry_count.max(1);
    let mut path_builder = StringBuilder::with_capacity(capacity, capacity * 32);
    let mut year_builder = Int64Builder::with_capacity(capacity);
    let mut month_builder = Int64Builder::with_capacity(capacity);
    let mut count_builder = Int64Builder::with_capacity(capacity);
    let mut min_id_builder = Int64Builder::with_capacity(capacity);
    let mut max_id_builder = Int64Builder::with_capacity(capacity);
    let mut min_ts_builder = Int64Builder::with_capacity(capacity);
    let mut max_ts_builder = Int64Builder::with_capacity(capacity);
    let mut file_size_builder = Int64Builder::with_capacity(capacity);

    for (key, stat) in entries {
        // Calculate total size for the partition directory
        let partition_path = key.dir_path();
        let full_partition_path = output_dir.join(&partition_path);

        let file_size_bytes = if full_partition_path.is_dir() {
            // Sum size of all files in directory
            fs::read_dir(&full_partition_path)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                        .sum()
                })
                .unwrap_or(0)
        } else {
            // Fallback for legacy single file
            output_dir
                .join(key.legacy_file_path())
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0)
        } as i64;

        path_builder.append_value(partition_path);
        year_builder.append_value(key.year as i64);
        month_builder.append_value(key.month as i64);
        count_builder.append_value(stat.changeset_count as i64);
        min_id_builder.append_value(stat.min_id);
        max_id_builder.append_value(stat.max_id);
        match stat.min_timestamp {
            Some(ts) => min_ts_builder.append_value(ts),
            None => min_ts_builder.append_null(),
        }
        match stat.max_timestamp {
            Some(ts) => max_ts_builder.append_value(ts),
            None => max_ts_builder.append_null(),
        }
        file_size_builder.append_value(file_size_bytes);
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(path_builder.finish()),
        Arc::new(year_builder.finish()),
        Arc::new(month_builder.finish()),
        Arc::new(count_builder.finish()),
        Arc::new(min_id_builder.finish()),
        Arc::new(max_id_builder.finish()),
        Arc::new(min_ts_builder.finish()),
        Arc::new(max_ts_builder.finish()),
        Arc::new(file_size_builder.finish()),
    ];

    let batch = RecordBatch::try_new(schema, columns)?;
    writer.write(&batch)?;
    writer.close()?;

    println!(
        "  Index partition_stats.parquet: {} partitions",
        entry_count
    );

    Ok(())
}

/// Write metadata.json file with partition information including ID ranges
pub fn write_metadata(
    output_dir: &PathBuf,
    total_count: usize,
    partition_counts: &HashMap<PartitionKey, usize>,
    partition_stats: &HashMap<PartitionKey, PartitionStats>,
) -> Result<()> {
    use std::io::Write;

    let metadata_path = output_dir.join("metadata.json");
    let mut file = File::create(&metadata_path)?;

    // Build partition list sorted by year/month
    let mut partitions: Vec<_> = partition_counts
        .iter()
        .map(|(k, v)| {
            let stats = partition_stats.get(k);
            serde_json::json!({
                "year": k.year,
                "month": k.month,
                "path": k.dir_path(),
                "count": v,
                "min_id": stats.map(|s| s.min_id),
                "max_id": stats.map(|s| s.max_id),
                "min_created_at": stats.and_then(|s| s.min_timestamp),
                "max_created_at": stats.and_then(|s| s.max_timestamp),
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
