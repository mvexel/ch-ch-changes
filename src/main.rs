use anyhow::{Context, Result};
use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int64Builder, MapBuilder, StringBuilder,
    TimestampMillisecondBuilder, UInt32Builder,
};
use arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use bzip2::read::MultiBzDecoder;
use changesets_to_parquet::Changeset;
use clap::Parser;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

const CHANNEL_BATCH_SIZE: usize = 1000;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input OSM changeset XML file (.osm or .osm.bz2). Use "-" for stdin.
    #[arg(short, long)]
    input: String,

    /// Output Parquet file (used when --partition is not set)
    #[arg(short, long)]
    output: Option<String>,

    /// Output directory for partitioned files (used with --partition)
    #[arg(long)]
    output_dir: Option<String>,

    /// Enable partitioned output by year/month
    #[arg(long, default_value_t = false)]
    partition: bool,

    /// Generate index files (user_stats.parquet, open_changesets.parquet, partition_stats.parquet)
    #[arg(long, default_value_t = false)]
    generate_indexes: bool,

    /// Batch size for writing records
    #[arg(short, long, default_value_t = 100000)]
    batch_size: usize,

    /// Continue processing on parse errors (saves what was successfully parsed)
    #[arg(long, default_value_t = false)]
    continue_on_error: bool,

    /// Number of worker threads for parallel writing (default: 1)
    #[arg(long, default_value_t = 1)]
    workers: usize,
}

struct BatchBuilders {
    id: Int64Builder,
    created_at: TimestampMillisecondBuilder,
    closed_at: TimestampMillisecondBuilder,
    open: BooleanBuilder,
    user: StringBuilder,
    uid: Int64Builder,
    min_lat: Float64Builder,
    min_lon: Float64Builder,
    max_lat: Float64Builder,
    max_lon: Float64Builder,
    num_changes: UInt32Builder,
    comments_count: UInt32Builder,
    description: StringBuilder,
    tags: MapBuilder<StringBuilder, StringBuilder>,
    len: usize,
}

impl BatchBuilders {
    fn with_capacity(capacity: usize) -> Self {
        // For string data we start with a modest byte capacity and Arrow will grow as needed.
        let string_byte_capacity = capacity * 16;

        Self {
            id: Int64Builder::with_capacity(capacity),
            created_at: TimestampMillisecondBuilder::with_capacity(capacity),
            closed_at: TimestampMillisecondBuilder::with_capacity(capacity),
            open: BooleanBuilder::with_capacity(capacity),
            user: StringBuilder::with_capacity(capacity, string_byte_capacity),
            uid: Int64Builder::with_capacity(capacity),
            min_lat: Float64Builder::with_capacity(capacity),
            min_lon: Float64Builder::with_capacity(capacity),
            max_lat: Float64Builder::with_capacity(capacity),
            max_lon: Float64Builder::with_capacity(capacity),
            num_changes: UInt32Builder::with_capacity(capacity),
            comments_count: UInt32Builder::with_capacity(capacity),
            description: StringBuilder::with_capacity(capacity, string_byte_capacity),
            tags: MapBuilder::with_capacity(
                None,
                StringBuilder::with_capacity(capacity * 2, string_byte_capacity),
                StringBuilder::with_capacity(capacity * 2, string_byte_capacity),
                capacity,
            ),
            len: 0,
        }
    }

    fn append(&mut self, cs: &Changeset) {
        self.id.append_value(cs.id);

        if let Some(ts) = cs.created_at {
            self.created_at.append_value(ts);
        } else {
            self.created_at.append_null();
        }

        if let Some(ts) = cs.closed_at {
            self.closed_at.append_value(ts);
        } else {
            self.closed_at.append_null();
        }

        self.open.append_value(cs.open);

        if let Some(ref user) = cs.user {
            self.user.append_value(user);
        } else {
            self.user.append_null();
        }

        if let Some(uid) = cs.uid {
            self.uid.append_value(uid);
        } else {
            self.uid.append_null();
        }

        if let Some(lat) = cs.min_lat {
            self.min_lat.append_value(lat);
        } else {
            self.min_lat.append_null();
        }

        if let Some(lon) = cs.min_lon {
            self.min_lon.append_value(lon);
        } else {
            self.min_lon.append_null();
        }

        if let Some(lat) = cs.max_lat {
            self.max_lat.append_value(lat);
        } else {
            self.max_lat.append_null();
        }

        if let Some(lon) = cs.max_lon {
            self.max_lon.append_value(lon);
        } else {
            self.max_lon.append_null();
        }

        self.num_changes.append_value(cs.num_changes);
        self.comments_count.append_value(cs.comments_count);

        if let Some(ref description) = cs.description {
            self.description.append_value(description);
        } else {
            self.description.append_null();
        }

        // Append tags
        if !cs.tags.is_empty() {
            for (k, v) in &cs.tags {
                self.tags.keys().append_value(k);
                self.tags.values().append_value(v);
            }
            self.tags.append(true).unwrap();
        } else {
            self.tags.append(true).unwrap();
        }

        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn finish_batch(&mut self, schema: &Arc<Schema>) -> Result<RecordBatch> {
        let columns: Vec<ArrayRef> = vec![
            Arc::new(self.id.finish()),
            Arc::new(self.created_at.finish()),
            Arc::new(self.closed_at.finish()),
            Arc::new(self.open.finish()),
            Arc::new(self.user.finish()),
            Arc::new(self.uid.finish()),
            Arc::new(self.min_lat.finish()),
            Arc::new(self.min_lon.finish()),
            Arc::new(self.max_lat.finish()),
            Arc::new(self.max_lon.finish()),
            Arc::new(self.num_changes.finish()),
            Arc::new(self.comments_count.finish()),
            Arc::new(self.description.finish()),
            Arc::new(self.tags.finish()),
        ];

        let batch = RecordBatch::try_new(schema.clone(), columns)?;
        self.len = 0;
        Ok(batch)
    }
}

/// Partition key for year/month partitioning
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct PartitionKey {
    year: i32,
    month: u32,
}

impl PartitionKey {
    fn from_timestamp_millis(ts: i64) -> Self {
        use chrono::{TimeZone, Utc};
        let dt = Utc.timestamp_millis_opt(ts).unwrap();
        Self {
            year: dt.year(),
            month: dt.month(),
        }
    }

    fn dir_path(&self) -> String {
        format!("year={}/month={:02}", self.year, self.month)
    }

    fn legacy_file_path(&self) -> String {
        format!("year={}/month={:02}.parquet", self.year, self.month)
    }
}

use chrono::Datelike;

/// Manages multiple Parquet writers, one per partition
struct PartitionedWriter {
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
    fn new(
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
            self.builders
                .insert(key.clone(), BatchBuilders::with_capacity(self.batch_size));
            self.partition_counts.insert(key.clone(), 0);
        }
        Ok(())
    }

    fn write_changeset(&mut self, cs: &Changeset) -> Result<()> {
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

    fn finish(mut self) -> Result<PartitionSummary> {
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
        use std::collections::hash_map::Entry;

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

#[derive(Debug, Clone, Default)]
struct PartitionStats {
    changeset_count: u64,
    min_id: i64,
    max_id: i64,
    min_timestamp: Option<i64>,
    max_timestamp: Option<i64>,
}

impl PartitionStats {
    fn from_changeset(cs: &Changeset) -> Self {
        Self {
            changeset_count: 1,
            min_id: cs.id,
            max_id: cs.id,
            min_timestamp: cs.created_at,
            max_timestamp: cs.created_at,
        }
    }

    fn update(&mut self, cs: &Changeset) {
        self.changeset_count = self.changeset_count.saturating_add(1);
        self.min_id = self.min_id.min(cs.id);
        self.max_id = self.max_id.max(cs.id);

        if let Some(ts) = cs.created_at {
            self.min_timestamp = match self.min_timestamp {
                Some(current) => Some(current.min(ts)),
                None => Some(ts),
            };
            self.max_timestamp = match self.max_timestamp {
                Some(current) => Some(current.max(ts)),
                None => Some(ts),
            };
        }
    }

    fn merge(&mut self, other: &PartitionStats) {
        self.changeset_count += other.changeset_count;
        self.min_id = self.min_id.min(other.min_id);
        self.max_id = self.max_id.max(other.max_id);
        
        if let Some(ts) = other.min_timestamp {
            self.min_timestamp = match self.min_timestamp {
                Some(current) => Some(current.min(ts)),
                None => Some(ts),
            };
        }
        if let Some(ts) = other.max_timestamp {
            self.max_timestamp = match self.max_timestamp {
                Some(current) => Some(current.max(ts)),
                None => Some(ts),
            };
        }
    }
}

struct PartitionSummary {
    counts: HashMap<PartitionKey, usize>,
    stats: HashMap<PartitionKey, PartitionStats>,
}

impl PartitionSummary {
    fn merge(&mut self, other: PartitionSummary) {
        for (key, count) in other.counts {
            *self.counts.entry(key.clone()).or_insert(0) += count;
        }
        for (key, stats) in other.stats {
            self.stats.entry(key).or_default().merge(&stats);
        }
    }
}

/// Tracks user statistics for index generation
#[derive(Debug, Default)]
struct UserStats {
    uid: i64,
    username: Option<String>,
    changeset_count: u64,
    first_changeset_at: Option<i64>,
    last_changeset_at: Option<i64>,
}

/// Collects data for generating indexes
struct IndexCollector {
    user_stats: HashMap<i64, UserStats>,
    open_changesets: Vec<Changeset>,
}

impl IndexCollector {
    fn new() -> Self {
        Self {
            user_stats: HashMap::new(),
            open_changesets: Vec::new(),
        }
    }

    fn record(&mut self, cs: &Changeset) {
        // Track open changesets
        if cs.open {
            self.open_changesets.push(Changeset {
                id: cs.id,
                created_at: cs.created_at,
                closed_at: cs.closed_at,
                open: cs.open,
                user: cs.user.clone(),
                uid: cs.uid,
                min_lat: cs.min_lat,
                min_lon: cs.min_lon,
                max_lat: cs.max_lat,
                max_lon: cs.max_lon,
                num_changes: cs.num_changes,
                comments_count: cs.comments_count,
                description: cs.description.clone(),
                tags: cs.tags.clone(),
            });
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
        }
    }

    fn write_indexes(self, output_dir: &PathBuf, schema: &Arc<Schema>) -> Result<()> {
        let indexes_dir = output_dir.join("indexes");
        fs::create_dir_all(&indexes_dir)?;

        // Write open changesets index
        self.write_open_changesets_index(&indexes_dir, schema)?;

        // Write user stats index
        self.write_user_stats_index(&indexes_dir)?;

        Ok(())
    }

    fn write_open_changesets_index(
        &self,
        indexes_dir: &PathBuf,
        schema: &Arc<Schema>,
    ) -> Result<()> {
        let file_path = indexes_dir.join("open_changesets.parquet");
        let file = File::create(&file_path)?;

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))?;
        let mut builders = BatchBuilders::with_capacity(self.open_changesets.len().max(1));

        for cs in &self.open_changesets {
            builders.append(cs);
        }

        if !builders.is_empty() {
            let batch = builders.finish_batch(schema)?;
            writer.write(&batch)?;
        }

        writer.close()?;
        println!(
            "  Index open_changesets.parquet: {} changesets",
            self.open_changesets.len()
        );

        Ok(())
    }

    fn write_user_stats_index(&self, indexes_dir: &PathBuf) -> Result<()> {
        let file_path = indexes_dir.join("user_stats.parquet");
        let file = File::create(&file_path)?;

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
        let mut count_builder = arrow::array::UInt64Builder::with_capacity(capacity);
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

        println!(
            "  Index user_stats.parquet: {} users",
            self.user_stats.len()
        );

        Ok(())
    }
}

fn parse_timestamp(s: &str) -> Result<i64> {
    let dt = chrono::DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("Failed to parse timestamp: {}", s))?;
    Ok(dt.timestamp_millis())
}

fn parse_changeset_element(e: &BytesStart) -> Result<Changeset> {
    let mut changeset = Changeset::default();

    for attr in e.attributes() {
        let attr = attr?;
        let key = attr.key.as_ref();
        let value = attr.unescape_value()?;

        match key {
            b"id" => changeset.id = value.as_ref().parse()?,
            b"created_at" => changeset.created_at = Some(parse_timestamp(value.as_ref())?),
            b"closed_at" => changeset.closed_at = Some(parse_timestamp(value.as_ref())?),
            b"open" => changeset.open = value == "true",
            b"user" => changeset.user = Some(value.to_string()),
            b"uid" => changeset.uid = Some(value.as_ref().parse()?),
            b"min_lat" => changeset.min_lat = Some(value.as_ref().parse()?),
            b"min_lon" => changeset.min_lon = Some(value.as_ref().parse()?),
            b"max_lat" => changeset.max_lat = Some(value.as_ref().parse()?),
            b"max_lon" => changeset.max_lon = Some(value.as_ref().parse()?),
            b"num_changes" => changeset.num_changes = value.as_ref().parse()?,
            b"comments_count" => changeset.comments_count = value.as_ref().parse()?,
            _ => {}
        }
    }

    Ok(changeset)
}

fn apply_changeset_tag(e: &BytesStart, changeset: &mut Changeset) -> Result<()> {
    let mut key = None;
    let mut value = None;

    for attr in e.attributes() {
        let attr = attr?;
        match attr.key.as_ref() {
            b"k" => key = Some(attr.unescape_value()?.into_owned()),
            b"v" => value = Some(attr.unescape_value()?.into_owned()),
            _ => {}
        }
    }

    if let (Some(k), Some(v)) = (key, value) {
        if k == "comment" {
            changeset.description = Some(v.clone());
        }
        changeset.tags.insert(k, v);
    }

    Ok(())
}

fn parse_changeset_body<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    changeset: &mut Changeset,
    buf: &mut Vec<u8>,
) -> Result<()> {
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"tag" {
                    apply_changeset_tag(e, changeset)?;
                }
            }
            Ok(Event::Start(e)) => {
                let owned = e.into_owned();
                let tag_name = owned.name().as_ref().to_vec();
                if owned.name().as_ref() == b"tag" {
                    apply_changeset_tag(&owned, changeset)?;
                }
                // Drop owned before handing mutable access back to reader/BufRead
                drop(owned);
                reader.read_to_end_into(QName(&tag_name), buf)?;
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"changeset" => break,
            Ok(Event::Eof) => {
                return Err(anyhow::anyhow!(
                    "Unexpected EOF while parsing changeset body"
                ))
            }
            Ok(_) => {} // Ignore other events
            Err(err) => return Err(err.into()),
        }
        buf.clear();
    }

    Ok(())
}

/// Producer: Parses XML and sends batches of changesets to the channel
fn parse_xml_to_channel<R: std::io::Read + Send + 'static>(
    reader: R,
    tx: SyncSender<Result<Vec<Changeset>>>,
    continue_on_error: bool,
) {
    // Use a large buffer (1MB) to avoid splitting XML tags across read boundaries
    let buffered_reader = BufReader::with_capacity(1024 * 1024, reader);
    let mut xml_reader = Reader::from_reader(buffered_reader);
    xml_reader.config_mut().trim_text_start = true;
    xml_reader.config_mut().trim_text_end = true;
    xml_reader.config_mut().check_end_names = false; // More lenient parsing
    xml_reader.config_mut().check_comments = false; // Don't validate comments

    let mut buf = Vec::new();
    let mut temp_buf = Vec::new();
    let mut last_changeset_id = 0i64;
    let mut count = 0;

    let mut batch = Vec::with_capacity(CHANNEL_BATCH_SIZE);

    loop {
        let position = xml_reader.buffer_position();
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"changeset" => {
                match parse_changeset_element(e) {
                    Ok(changeset) => {
                        last_changeset_id = changeset.id;
                        batch.push(changeset);
                        count += 1;
                    }
                    Err(e) => {
                        if !continue_on_error {
                            let _ = tx.send(Err(e));
                            return;
                        } else {
                            eprintln!("Error parsing changeset element at pos {}: {}", position, e);
                        }
                    }
                }
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"changeset" => {
                match parse_changeset_element(e) {
                    Ok(mut changeset) => {
                        temp_buf.clear();
                        match parse_changeset_body(&mut xml_reader, &mut changeset, &mut temp_buf) {
                            Ok(_) => {
                                last_changeset_id = changeset.id;
                                batch.push(changeset);
                                count += 1;
                            }
                            Err(e) => {
                                if !continue_on_error {
                                    let _ = tx.send(Err(e));
                                    return;
                                } else {
                                    eprintln!(
                                        "Error parsing changeset body for ID {}: {}",
                                        changeset.id, e
                                    );
                                }
                            }
                        }
                        temp_buf.clear();
                    }
                    Err(e) => {
                        if !continue_on_error {
                            let _ = tx.send(Err(e));
                            return;
                        } else {
                            eprintln!("Error parsing changeset element at pos {}: {}", position, e);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                eprintln!("\n=== XML PARSE ERROR ===");
                eprintln!("Position: {}", position);
                eprintln!("Changesets processed: {}", count);
                eprintln!("Last changeset ID: {}", last_changeset_id);
                eprintln!("Error: {}", e);
                eprintln!("\nBuffer content at error (first 500 bytes):");
                eprintln!("{}", String::from_utf8_lossy(&buf[..buf.len().min(500)]));
                eprintln!("\nBuffer content at error (last 500 bytes):");
                let start = if buf.len() > 500 { buf.len() - 500 } else { 0 };
                eprintln!("{}", String::from_utf8_lossy(&buf[start..]));
                eprintln!("======================\n");

                if continue_on_error {
                    eprintln!(
                        "Continuing with {} successfully parsed changesets...",
                        count
                    );
                    break;
                } else {
                    let _ = tx.send(Err(anyhow::anyhow!(
                        "Error parsing XML: {}. Use --continue-on-error to save partial results.",
                        e
                    )));
                    return;
                }
            }
            _ => {} // Ignore other events
        }
        buf.clear();

        if batch.len() >= CHANNEL_BATCH_SIZE {
            if tx.send(Ok(batch)).is_err() {
                // Receiver disconnected
                return;
            }
            batch = Vec::with_capacity(CHANNEL_BATCH_SIZE);
        }
    }

    if !batch.is_empty() {
        let _ = tx.send(Ok(batch));
    }
}

/// Consumer: Writes flat parquet file
fn consume_and_write_flat(
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
fn consume_and_write_partitioned_parallel(
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
                    collector.record(&cs);
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
                collector.record(cs);
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

fn create_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            true,
        ),
        Field::new(
            "closed_at",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            true,
        ),
        Field::new("open", DataType::Boolean, false),
        Field::new("user", DataType::Utf8, true),
        Field::new("uid", DataType::Int64, true),
        Field::new("min_lat", DataType::Float64, true),
        Field::new("min_lon", DataType::Float64, true),
        Field::new("max_lat", DataType::Float64, true),
        Field::new("max_lon", DataType::Float64, true),
        Field::new("num_changes", DataType::UInt32, false),
        Field::new("comments_count", DataType::UInt32, false),
        Field::new("description", DataType::Utf8, true),
        Field::new(
            "tags",
            DataType::Map(
                Arc::new(Field::new(
                    "entries",
                    DataType::Struct(Fields::from(vec![ 
                        Field::new("keys", DataType::Utf8, false),
                        Field::new("values", DataType::Utf8, true),
                    ])),
                    false,
                )),
                false,
            ),
            false,
        ),
    ]))
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate arguments
    if args.partition {
        if args.output_dir.is_none() {
            return Err(anyhow::anyhow!(
                "--output-dir is required when using --partition"
            ));
        }
    } else if args.output.is_none() {
        return Err(anyhow::anyhow!(
            "--output is required when not using --partition"
        ));
    }

    println!("Reading from: {}", args.input);
    if args.workers > 1 && args.partition {
        println!("Using {} worker threads for sharded output", args.workers);
    }

    let schema = create_schema();
    
    let (tx, rx) = sync_channel(32);
    let continue_on_error = args.continue_on_error;
    
    // Determine input source and compression
    let is_stdin = args.input == "-";
    let is_bz2 = args.input.ends_with(".bz2");
    
    let input_source: Box<dyn Read + Send> = if is_stdin {
        Box::new(std::io::stdin())
    } else {
        Box::new(File::open(&args.input)
            .with_context(|| format!("Failed to open input file: {}", args.input))?) 
    };

    // Spawn producer thread
    let parser_handle = thread::spawn(move || {
        if is_bz2 && !is_stdin {
            // Only try internal decompression if it's a file with .bz2 extension
            // For stdin, we assume external decompression (pbzip2) or uncompressed stream
            // unless the user is piping a bz2 file directly, in which case we might fail to detect.
            // But standard usage with pipe is `pbzip2 -dc ... | tool`.
            println!("Detected bzip2 multi-stream compressed file (internal decompression)");
            let decoder = MultiBzDecoder::new(input_source);
            parse_xml_to_channel(decoder, tx, continue_on_error);
        } else {
            // Treat as plain XML stream (could be coming from pbzip2 pipe)
            if is_stdin {
                println!("Reading from stdin (assuming decompressed XML stream)");
            }
            parse_xml_to_channel(input_source, tx, continue_on_error);
        }
    });

    if args.partition {
        // Partitioned output mode
        let output_dir = PathBuf::from(args.output_dir.as_ref().unwrap());
        println!("Writing partitioned output to: {:?}", output_dir);

        let mut index_collector = if args.generate_indexes {
            println!("Index generation enabled");
            Some(IndexCollector::new())
        } else {
            None
        };

        let (total_count, summary) = consume_and_write_partitioned_parallel(
            rx,
            output_dir.clone(),
            schema.clone(),
            args.batch_size,
            args.workers,
            index_collector.as_mut(),
        )?;

        println!("\nFinalizing partitions...");
        let PartitionSummary {
            counts: partition_counts,
            stats: partition_stats,
        } = summary;

        if let Some(collector) = index_collector {
            println!("\nWriting indexes...");
            collector.write_indexes(&output_dir, &schema)?;
            write_partition_stats_index(&output_dir, &partition_stats)?;
        }

        println!(
            "\nSuccessfully wrote {} changesets to {} partitions",
            total_count,
            partition_counts.len()
        );

        // Write metadata file
        write_metadata(&output_dir, total_count, &partition_counts)?;
    } else {
        // Single file output mode (original behavior)
        let output = args.output.as_ref().unwrap();
        println!("Writing to: {}", output);

        let output_file = File::create(output)
            .with_context(|| format!("Failed to create output file: {}", output))?;

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        let mut writer = ArrowWriter::try_new(output_file, schema.clone(), Some(props))?;

        let total_count = consume_and_write_flat(rx, &mut writer, &schema, args.batch_size)?;

        writer.close()?;

        println!(
            "Successfully wrote {} changesets to {}",
            total_count, output
        );
    }

    parser_handle.join().unwrap();

    Ok(())
}

fn write_partition_stats_index(
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
        } else { // Fallback for legacy single file
             output_dir.join(key.legacy_file_path())
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

/// Write metadata.json file with partition information
fn write_metadata(
    output_dir: &PathBuf,
    total_count: usize,
    partition_counts: &HashMap<PartitionKey, usize>,
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
