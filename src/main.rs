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
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input OSM changeset XML file (.osm or .osm.bz2)
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

    fn path(&self) -> String {
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
    writers: HashMap<PartitionKey, ArrowWriter<File>>,
    builders: HashMap<PartitionKey, BatchBuilders>,
    partition_counts: HashMap<PartitionKey, usize>,
    partition_stats: HashMap<PartitionKey, PartitionStats>,
}

impl PartitionedWriter {
    fn new(output_dir: PathBuf, schema: Arc<Schema>, batch_size: usize) -> Result<Self> {
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
            // Create directory for this partition
            let partition_dir = self.output_dir.join(format!("year={}", key.year));
            fs::create_dir_all(&partition_dir)?;

            let file_path = self.output_dir.join(key.path());
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
            println!(
                "  Partition {}: {} changesets",
                key.path(),
                self.partition_counts.get(&key).unwrap_or(&0)
            );
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
}

struct PartitionSummary {
    counts: HashMap<PartitionKey, usize>,
    stats: HashMap<PartitionKey, PartitionStats>,
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
            Ok(_) => {}
            Err(err) => return Err(err.into()),
        }
        buf.clear();
    }

    Ok(())
}

fn parse_and_write_changesets<R: std::io::Read>(
    reader: R,
    writer: &mut ArrowWriter<File>,
    schema: &Arc<Schema>,
    batch_size: usize,
    continue_on_error: bool,
) -> Result<usize> {
    // Use a large buffer (1MB) to avoid splitting XML tags across read boundaries
    let buffered_reader = BufReader::with_capacity(1024 * 1024, reader);
    let mut xml_reader = Reader::from_reader(buffered_reader);
    xml_reader.config_mut().trim_text_start = true;
    xml_reader.config_mut().trim_text_end = true;
    xml_reader.config_mut().check_end_names = false; // More lenient parsing
    xml_reader.config_mut().check_comments = false; // Don't validate comments

    let mut buf = Vec::new();
    let mut count = 0;
    let mut batch_num = 0;
    let mut last_changeset_id = 0i64;

    let mut temp_buf = Vec::new();
    let effective_batch_size = batch_size.max(1);
    let mut builders = BatchBuilders::with_capacity(effective_batch_size);

    let mut process_changeset = |changeset: Changeset| -> Result<()> {
        last_changeset_id = changeset.id;
        builders.append(&changeset);
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

        Ok(())
    };

    loop {
        let position = xml_reader.buffer_position();
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"changeset" => {
                let changeset = parse_changeset_element(e)?;
                process_changeset(changeset)?;
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"changeset" => {
                let mut changeset = parse_changeset_element(e)?;
                temp_buf.clear();
                parse_changeset_body(&mut xml_reader, &mut changeset, &mut temp_buf)?;
                temp_buf.clear();
                process_changeset(changeset)?;
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
                    return Err(anyhow::anyhow!(
                        "Error parsing XML: {}. Use --continue-on-error to save partial results.",
                        e
                    ));
                }
            }
            _ => {}
        }
        buf.clear();
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

/// Parse and write changesets to partitioned output
fn parse_and_write_partitioned<R: std::io::Read>(
    reader: R,
    partitioned_writer: &mut PartitionedWriter,
    mut index_collector: Option<&mut IndexCollector>,
    continue_on_error: bool,
) -> Result<usize> {
    let buffered_reader = BufReader::with_capacity(1024 * 1024, reader);
    let mut xml_reader = Reader::from_reader(buffered_reader);
    xml_reader.config_mut().trim_text_start = true;
    xml_reader.config_mut().trim_text_end = true;
    xml_reader.config_mut().check_end_names = false;
    xml_reader.config_mut().check_comments = false;

    let mut buf = Vec::new();
    let mut temp_buf = Vec::new();
    let mut count = 0;
    let mut last_changeset_id = 0i64;

    loop {
        let position = xml_reader.buffer_position();
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"changeset" => {
                let changeset = parse_changeset_element(e)?;
                last_changeset_id = changeset.id;

                if let Some(ref mut collector) = index_collector.as_deref_mut() {
                    collector.record(&changeset);
                }

                partitioned_writer.write_changeset(&changeset)?;
                count += 1;

                if count % 1_000_000 == 0 {
                    println!("Processed {} changesets...", count);
                }
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"changeset" => {
                let mut changeset = parse_changeset_element(e)?;
                temp_buf.clear();
                parse_changeset_body(&mut xml_reader, &mut changeset, &mut temp_buf)?;
                temp_buf.clear();

                last_changeset_id = changeset.id;

                if let Some(ref mut collector) = index_collector.as_deref_mut() {
                    collector.record(&changeset);
                }

                partitioned_writer.write_changeset(&changeset)?;
                count += 1;

                if count % 1_000_000 == 0 {
                    println!("Processed {} changesets...", count);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                eprintln!("\n=== XML PARSE ERROR ===");
                eprintln!("Position: {}", position);
                eprintln!("Changesets processed: {}", count);
                eprintln!("Last changeset ID: {}", last_changeset_id);
                eprintln!("Error: {}", e);
                eprintln!("======================\n");

                if continue_on_error {
                    eprintln!(
                        "Continuing with {} successfully parsed changesets...",
                        count
                    );
                    break;
                } else {
                    return Err(anyhow::anyhow!(
                        "Error parsing XML: {}. Use --continue-on-error to save partial results.",
                        e
                    ));
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(count)
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
                        Field::new("key", DataType::Utf8, false),
                        Field::new("value", DataType::Utf8, true),
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

    let schema = create_schema();

    // Open input file
    let file = File::open(&args.input)
        .with_context(|| format!("Failed to open input file: {}", args.input))?;

    if args.partition {
        // Partitioned output mode
        let output_dir = PathBuf::from(args.output_dir.as_ref().unwrap());
        println!("Writing partitioned output to: {:?}", output_dir);

        let mut partitioned_writer =
            PartitionedWriter::new(output_dir.clone(), schema.clone(), args.batch_size)?;

        let mut index_collector = if args.generate_indexes {
            println!("Index generation enabled");
            Some(IndexCollector::new())
        } else {
            None
        };

        let total_count = if args.input.ends_with(".bz2") {
            println!("Detected bzip2 multi-stream compressed file");
            let decoder = MultiBzDecoder::new(file);
            parse_and_write_partitioned(
                decoder,
                &mut partitioned_writer,
                index_collector.as_mut(),
                args.continue_on_error,
            )?
        } else {
            parse_and_write_partitioned(
                file,
                &mut partitioned_writer,
                index_collector.as_mut(),
                args.continue_on_error,
            )?
        };

        println!("\nFinalizing partitions...");
        let PartitionSummary {
            counts: partition_counts,
            stats: partition_stats,
        } = partitioned_writer.finish()?;

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

        let total_count = if args.input.ends_with(".bz2") {
            println!("Detected bzip2 multi-stream compressed file");
            let decoder = MultiBzDecoder::new(file);
            parse_and_write_changesets(
                decoder,
                &mut writer,
                &schema,
                args.batch_size,
                args.continue_on_error,
            )?
        } else {
            parse_and_write_changesets(
                file,
                &mut writer,
                &schema,
                args.batch_size,
                args.continue_on_error,
            )?
        };

        writer.close()?;

        println!(
            "Successfully wrote {} changesets to {}",
            total_count, output
        );
    }

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
        let path = key.path();
        let file_size_bytes = output_dir
            .join(&path)
            .metadata()
            .map(|m| m.len() as i64)
            .unwrap_or(0);

        path_builder.append_value(path);
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
                "path": k.path(),
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
