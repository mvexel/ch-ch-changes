use anyhow::{Context, Result};
use bzip2::read::MultiBzDecoder;
use changesets_to_parquet::{
    indexing::{write_metadata, write_partition_stats_index, IndexCollector},
    models::PartitionSummary,
    parsing::parse_xml_to_channel,
    schema::create_schema,
    writer::{consume_and_write_flat, consume_and_write_partitioned_parallel},
};
use clap::Parser;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::thread;

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
    #[arg(long, default_value_t = 10000, hide = true)]
    batch_size: usize,

    /// Continue processing on parse errors (saves what was successfully parsed)
    #[arg(long, default_value_t = false)]
    continue_on_error: bool,

    /// Number of worker threads for parallel writing (default: 1)
    #[arg(long, default_value_t = 1, hide = true)]
    workers: usize,
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
            Some(IndexCollector::new(&output_dir, schema.clone())?)
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
            collector.write_indexes()?;
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
            total_count,
            output
        );
    }

    parser_handle.join().unwrap();

    Ok(())
}
