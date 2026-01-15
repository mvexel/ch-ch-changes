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
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;
use std::thread;

/// Initial capacity for BatchBuilders - kept small to avoid memory bloat with many partitions.
const PARTITION_BUILDER_INITIAL_CAPACITY: usize = 4096;

/// Manages multiple Parquet writers, one per partition
pub struct PartitionedWriter {
    output_dir: PathBuf,
    schema: Arc<Schema>,
    props: WriterProperties,
    batch_size: usize,
    shard_id: usize,
    use_sharding: bool,
    writers: HashMap<PartitionKey, ArrowWriter<File>>,
    builders: HashMap<PartitionKey, BatchBuilders>,
    partition_counts: HashMap<PartitionKey, usize>,
    partition_stats: HashMap<PartitionKey, PartitionStats>,
}

impl PartitionedWriter {
    pub fn new(
        output_dir: PathBuf,
        schema: Arc<Schema>,
        batch_size: usize,
        shard_id: usize,
        use_sharding: bool,
    ) -> Result<Self> {
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
            shard_id,
            use_sharding,
            writers: HashMap::new(),
            builders: HashMap::new(),
            partition_counts: HashMap::new(),
            partition_stats: HashMap::new(),
        })
    }

    fn get_or_create_writer(&mut self, key: &PartitionKey) -> Result<()> {
        if !self.writers.contains_key(key) {
            let file_path = if self.use_sharding {
                // New directory structure: year=Y/month=M/part-{shard}.parquet
                let partition_dir = self.output_dir.join(key.dir_path());
                fs::create_dir_all(&partition_dir)?;
                partition_dir.join(format!("part-{:05}.parquet", self.shard_id))
            } else {
                // Legacy structure: year=Y/month=M.parquet
                // Only safe if single threaded, but we support single thread worker without sharding enabled.
                let partition_dir = self.output_dir.join(format!("year={}", key.year));
                fs::create_dir_all(&partition_dir)?;
                self.output_dir.join(key.legacy_file_path())
            };
            
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
            if !self.use_sharding || self.shard_id == 0 {
                 println!(
                    "  [Shard {}] Partition {}: {} changesets",
                    self.shard_id,
                    key.dir_path(),
                    self.partition_counts.get(&key).unwrap_or(&0)
                );
            }
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
    num_workers: usize,
    mut index_collector: Option<&mut IndexCollector>,
) -> Result<(usize, PartitionSummary)> {
    
    if num_workers <= 1 {
        // Single threaded mode (legacy behavior compatible)
        let mut partitioned_writer = PartitionedWriter::new(
            output_dir,
            schema,
            batch_size,
            0,
            false // No sharding in file names for single worker
        )?;
        
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
        return Ok((count, summary));
    }
    
    // Multi-threaded mode
    let mut worker_senders = Vec::with_capacity(num_workers);
    let mut worker_handles = Vec::with_capacity(num_workers);

    for i in 0..num_workers {
        let (tx, worker_rx) = sync_channel::<Vec<Changeset>>(32);
        worker_senders.push(tx);
        
        let worker_output_dir = output_dir.clone();
        let worker_schema = schema.clone();
        
        let handle = thread::spawn(move || -> Result<PartitionSummary> {
            let mut writer = PartitionedWriter::new(
                worker_output_dir, 
                worker_schema, 
                batch_size, 
                i,
                true // Use sharding
            )?;
            
            for batch in worker_rx {
                for cs in batch {
                    writer.write_changeset(&cs)?;
                }
            }
            
            writer.finish()
        });
        
        worker_handles.push(handle);
    }
    
    // Dispatcher
    let mut count = 0;
    let mut worker_idx = 0;
    let mut dispatcher_error = None;
    
    for batch_result in rx {
        // Check if we encountered an upstream error (parser)
        let batch = match batch_result {
            Ok(b) => b,
            Err(e) => {
                dispatcher_error = Some(e);
                break;
            }
        };
        
        // Record indexes in the main thread
        if let Some(ref mut collector) = index_collector.as_deref_mut() {
            for cs in &batch {
                collector.record(cs)?;
            }
        }
        
        count += batch.len();
        if count % 1_000_000 < batch.len() {
             println!("Processed {} changesets...", count);
        }
        
        // Round-robin dispatch
        if let Err(_) = worker_senders[worker_idx].send(batch) {
            // The worker has died (dropped its receiver). 
            // We break the loop to collect the actual error from the worker threads.
            eprintln!("Worker thread {} disconnected unexpectedly. collecting errors...", worker_idx);
            break;
        }
        worker_idx = (worker_idx + 1) % num_workers;
    }
    
    // Drop senders to signal completion to active workers
    drop(worker_senders);
    
    // Collect results and check for errors
    let mut aggregated_summary = PartitionSummary {
        counts: HashMap::new(),
        stats: HashMap::new(),
    };
    
    // If we had a dispatcher error (e.g. parser failed), return it
    if let Some(e) = dispatcher_error {
        // Wait for workers to finish what they have
        for handle in worker_handles {
            let _ = handle.join();
        }
        return Err(e);
    }

    // Collect worker results
    for (i, handle) in worker_handles.into_iter().enumerate() {
        match handle.join() {
            Ok(result) => {
                match result {
                    Ok(summary) => aggregated_summary.merge(summary),
                    Err(e) => {
                        // This is likely the "Too many open files" or similar error
                        return Err(e.context(format!("Worker {} failed", i)));
                    }
                }
            }
            Err(e) => {
                // Thread panicked
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Unknown panic".to_string()
                };
                return Err(anyhow::anyhow!("Worker {} panicked: {}", i, msg));
            }
        }
    }
    
    Ok((count, aggregated_summary))
}
