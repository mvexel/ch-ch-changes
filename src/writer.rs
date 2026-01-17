use crate::builder::BatchBuilders;
use crate::indexing::IndexCollector;
use crate::models::{PartitionKey, PartitionStats, PartitionSummary};
use crate::Changeset;
use anyhow::{Context, Result};
use arrow::datatypes::Schema;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

/// Initial capacity for BatchBuilders - kept small to avoid memory bloat with many partitions.
const PARTITION_BUILDER_INITIAL_CAPACITY: usize = 4096;

/// Manages multiple Parquet writers, one per partition
pub struct PartitionedWriter {
    output_dir: PathBuf,
    schema: Arc<Schema>,
    props: WriterProperties,
    batch_size: usize,
    writers: HashMap<PartitionKey, ArrowWriter<File>>,
    builders: HashMap<PartitionKey, BatchBuilders>,
    partition_counts: HashMap<PartitionKey, usize>,
    partition_stats: HashMap<PartitionKey, PartitionStats>,
}

impl PartitionedWriter {
    pub fn new(output_dir: PathBuf, schema: Arc<Schema>, batch_size: usize) -> Result<Self> {
        // Create output directory structure
        fs::create_dir_all(&output_dir)
            .with_context(|| format!("Failed to create output directory: {:?}", output_dir))?;

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        Ok(Self {
            output_dir,
            schema,
            props,
            batch_size,
            writers: HashMap::new(),
            builders: HashMap::new(),
            partition_counts: HashMap::new(),
            partition_stats: HashMap::new(),
        })
    }

    fn get_or_create_writer(&mut self, key: &PartitionKey) -> Result<()> {
        if !self.writers.contains_key(key) {
            // Legacy structure: year=Y/month=M.parquet
            let partition_dir = self.output_dir.join(format!("year={}", key.year));
            fs::create_dir_all(&partition_dir)?;
            let file_path = self.output_dir.join(key.legacy_file_path());
            
            let file = File::create(&file_path)
                .with_context(|| format!("Failed to create partition file: {:?}", file_path))?;

            let writer = ArrowWriter::try_new(file, self.schema.clone(), Some(self.props.clone()))?;
            self.writers.insert(key.clone(), writer);
            // Use small initial capacity - Arrow grows as needed. batch_size only controls flush threshold.
            self.builders
                .insert(key.clone(), BatchBuilders::with_capacity(PARTITION_BUILDER_INITIAL_CAPACITY));
            self.partition_counts.insert(key.clone(), 0);
        }
        Ok(())
    }

    pub fn write_changeset(&mut self, cs: &Changeset) -> Result<()> {
        // Determine partition key from created_at timestamp
        let key = match cs.created_at {
            Some(ts) => PartitionKey::from_timestamp_millis(ts),
            None => {
                // Changesets without created_at go to year=0/month=00
                PartitionKey { year: 0, month: 0 }
            }
        };

        self.get_or_create_writer(&key)?;
        self.update_partition_stats(&key, cs);

        let builders = self.builders.get_mut(&key).unwrap();
        builders.append(cs);
        *self.partition_counts.get_mut(&key).unwrap() += 1;

        // Flush if batch is full
        if builders.len() >= self.batch_size {
            let batch = builders.finish_batch(&self.schema)?;
            let writer = self.writers.get_mut(&key).unwrap();
            writer.write(&batch)?;
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<PartitionSummary> {
        // Flush remaining data in all partitions
        for (key, mut builders) in self.builders.drain() {
            if !builders.is_empty() {
                let batch = builders.finish_batch(&self.schema)?;
                if let Some(writer) = self.writers.get_mut(&key) {
                    writer.write(&batch)?;
                }
            }
        }

        // Close all writers
        for (key, writer) in self.writers.drain() {
            writer.close()?;
            println!(
                "  Partition {}: {} changesets",
                key.dir_path(),
                self.partition_counts.get(&key).unwrap_or(&0)
            );
        }

        Ok(PartitionSummary {
            counts: self.partition_counts,
            stats: self.partition_stats,
        })
    }

    fn update_partition_stats(&mut self, key: &PartitionKey, cs: &Changeset) {
        match self.partition_stats.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(PartitionStats::from_changeset(cs));
            }
            Entry::Occupied(mut entry) => {
                entry.get_mut().update(cs);
            }
        }
    }
}

/// Consumer: Writes flat parquet file
pub fn consume_and_write_flat(
    rx: Receiver<Result<Vec<Changeset>>>,
    writer: &mut ArrowWriter<File>,
    schema: &Arc<Schema>,
    batch_size: usize,
) -> Result<usize> {
    let mut count = 0;
    let mut batch_num = 0;
    
    let effective_batch_size = batch_size.max(1);
    let mut builders = BatchBuilders::with_capacity(effective_batch_size);

    for batch_result in rx {
        let batch = batch_result?;
        for cs in batch {
            builders.append(&cs);
            count += 1;

            if builders.len() >= effective_batch_size {
                batch_num += 1;
                let batch = builders.finish_batch(schema)?;
                let rows = batch.num_rows();
                println!(
                    "Writing batch {} with {} rows (total: {})...",
                    batch_num, rows, count
                );
                writer.write(&batch)?;
            }
        }
    }

    // Process remaining changesets
    if !builders.is_empty() {
        batch_num += 1;
        let batch = builders.finish_batch(schema)?;
        let rows = batch.num_rows();
        println!(
            "Writing final batch {} with {} rows (total: {})...",
            batch_num, rows, count
        );
        writer.write(&batch)?;
    }

    Ok(count)
}

/// Consumer: Writes partitioned parquet files (single or multi-threaded)
pub fn consume_and_write_partitioned_parallel(
    rx: Receiver<Result<Vec<Changeset>>>,
    output_dir: PathBuf,
    schema: Arc<Schema>,
    batch_size: usize,
    mut index_collector: Option<&mut IndexCollector>,
) -> Result<(usize, PartitionSummary)> {
    let mut partitioned_writer = PartitionedWriter::new(output_dir, schema, batch_size)?;
    let mut count = 0;

    for batch_result in rx {
        let batch = batch_result?;
        for cs in batch {
            if let Some(ref mut collector) = index_collector.as_deref_mut() {
                collector.record(&cs)?;
            }
            partitioned_writer.write_changeset(&cs)?;
            count += 1;

            if count % 1_000_000 == 0 {
                println!("Processed {} changesets...", count);
            }
        }
    }

    let summary = partitioned_writer.finish()?;
    Ok((count, summary))
}
