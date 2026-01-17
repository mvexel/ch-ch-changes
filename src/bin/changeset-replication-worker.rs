use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use clap::Parser;
use changesets_to_parquet::{
    parsing::parse_xml_to_channel, schema::create_schema, writer::consume_and_write_partitioned_parallel,
};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Root data path (contains base/ and delta/ or a flat layout)
    #[arg(long, env = "DATA_PATH", default_value = "/opt/changeset-data/changesets")]
    data_path: String,

    /// Delta output directory (default: DATA_PATH/delta)
    #[arg(long, env = "DELTA_DIR")]
    delta_dir: Option<String>,

    /// Local replication state file (default: DELTA_DIR/state.yaml)
    #[arg(long, env = "REPLICATION_STATE_FILE")]
    state_file: Option<String>,

    /// Temporary working directory (default: DELTA_DIR/tmp)
    #[arg(long, env = "REPLICATION_TMP_DIR")]
    temp_dir: Option<String>,

    /// Replication base URL (minutely changesets)
    #[arg(
        long,
        env = "CHANGESET_REPLICATION_URL",
        default_value = "https://planet.openstreetmap.org/replication/changesets/"
    )]
    replication_url: String,

    /// Poll interval in seconds when caught up
    #[arg(long, env = "POLL_INTERVAL_SECONDS", default_value_t = 60)]
    poll_interval_seconds: u64,

    /// Maximum sequences to process per run (useful for controlled backfill)
    #[arg(long, env = "MAX_SEQUENCES_PER_RUN")]
    max_sequences_per_run: Option<u64>,

    /// Sequence number to start from if no local state exists
    #[arg(long, env = "REPLICATION_START_SEQUENCE")]
    start_sequence: Option<u64>,

    /// Start from the first sequence at/after this timestamp (RFC3339) if no local state exists
    #[arg(long, env = "REPLICATION_START_TIMESTAMP")]
    start_timestamp: Option<String>,

    /// Exit after a single catch-up pass
    #[arg(long, default_value_t = false)]
    once: bool,

    /// Initialize local state from partition stats and exit
    #[arg(long, default_value_t = false)]
    bootstrap_state_from_parquet: bool,

    /// Batch size for writing records
    #[arg(long, default_value_t = 10000, hide = true)]
    batch_size: usize,

    /// Continue processing on parse errors (saves what was successfully parsed)
    #[arg(long, default_value_t = false)]
    continue_on_error: bool,

    /// Keep temporary files for debugging
    #[arg(long, default_value_t = false)]
    keep_temp: bool,
}

#[derive(Debug, Clone)]
struct ReplicationState {
    sequence_number: u64,
    timestamp: DateTime<Utc>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let data_path = PathBuf::from(&args.data_path);
    let delta_dir = args
        .delta_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| data_path.join("delta"));
    let state_path = args
        .state_file
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| delta_dir.join("state.yaml"));
    let temp_root = args
        .temp_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| delta_dir.join("tmp"));

    fs::create_dir_all(&delta_dir)
        .with_context(|| format!("Failed to create delta dir at {:?}", delta_dir))?;
    fs::create_dir_all(&temp_root)
        .with_context(|| format!("Failed to create temp dir at {:?}", temp_root))?;

    let client = build_client()?;
    let base_url = normalize_base_url(&args.replication_url);

    if args.bootstrap_state_from_parquet {
        bootstrap_state_from_parquet(&client, &base_url, &data_path, &state_path, &delta_dir)?;
        return Ok(());
    }

    let mut last_state = load_local_state(&state_path)?;
    if last_state.is_none() {
        last_state = Some(initialize_state(&client, &base_url, &args)?);
    }
    let mut last_state = last_state.expect("state must be initialized");

    loop {
        let remote_state = fetch_root_state(&client, &base_url)?;
        if last_state.sequence_number >= remote_state.sequence_number {
            if args.once {
                break;
            }
            sleep_poll(args.poll_interval_seconds);
            continue;
        }

        let mut end_sequence = remote_state.sequence_number;
        if let Some(max_per_run) = args.max_sequences_per_run {
            end_sequence = end_sequence.min(last_state.sequence_number + max_per_run);
        }

        for seq in (last_state.sequence_number + 1)..=end_sequence {
            log_info(&format!("Processing sequence {}", seq));
            let seq_state = process_sequence(
                &client,
                &base_url,
                seq,
                &temp_root,
                &delta_dir,
                &args,
            )?;
            write_state(&state_path, &seq_state)?;
            last_state = seq_state;
        }

        if args.once {
            break;
        }
    }

    Ok(())
}

fn build_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(format!(
            "changeset-replication-worker/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("Failed to build HTTP client")
}

fn normalize_base_url(url: &str) -> String {
    if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{}/", url)
    }
}

fn sleep_poll(seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds.max(1)));
}

fn initialize_state(client: &Client, base_url: &str, args: &Args) -> Result<ReplicationState> {
    if let Some(seq) = args.start_sequence {
        let state = fetch_sequence_state(client, base_url, seq)?
            .unwrap_or_else(|| ReplicationState {
                sequence_number: seq,
                timestamp: Utc::now(),
            });
        return Ok(ReplicationState {
            sequence_number: seq.saturating_sub(1),
            timestamp: state.timestamp,
        });
    }

    if let Some(ts) = &args.start_timestamp {
        let target = DateTime::parse_from_rfc3339(ts)
            .with_context(|| format!("Invalid start timestamp: {}", ts))?
            .with_timezone(&Utc);
        let seq = resolve_sequence_for_timestamp(client, base_url, target)?;
        let state = fetch_sequence_state(client, base_url, seq)?
            .unwrap_or_else(|| ReplicationState {
                sequence_number: seq,
                timestamp: target,
            });
        return Ok(ReplicationState {
            sequence_number: seq.saturating_sub(1),
            timestamp: state.timestamp,
        });
    }

    let latest = fetch_root_state(client, base_url)?;
    Ok(ReplicationState {
        sequence_number: latest.sequence_number,
        timestamp: latest.timestamp,
    })
}

fn load_local_state(path: &Path) -> Result<Option<ReplicationState>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read state file at {:?}", path))?;
    let state = parse_state(&text)
        .with_context(|| format!("Failed to parse state file at {:?}", path))?;
    Ok(Some(state))
}

fn parse_state(text: &str) -> Result<ReplicationState> {
    let mut sequence: Option<u64> = None;
    let mut timestamp: Option<DateTime<Utc>> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "---" {
            continue;
        }
        if let Some(value) = line.strip_prefix("sequenceNumber=") {
            sequence = Some(value.parse::<u64>()?);
        } else if let Some(value) = line.strip_prefix("sequence:") {
            sequence = Some(value.trim().parse::<u64>()?);
        } else if let Some(value) = line.strip_prefix("timestamp=") {
            timestamp = Some(parse_timestamp(value)?);
        } else if let Some(value) = line.strip_prefix("last_run:") {
            timestamp = Some(parse_timestamp(value)?);
        }
    }

    let sequence_number = sequence.context("state missing sequenceNumber")?;
    let ts = timestamp.context("state missing timestamp")?;
    Ok(ReplicationState {
        sequence_number,
        timestamp: ts,
    })
}

fn write_state(path: &Path, state: &ReplicationState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)
        .with_context(|| format!("Failed to write state file at {:?}", path))?;
    writeln!(file, "---")?;
    writeln!(
        file,
        "last_run: {}",
        state.timestamp.format("%Y-%m-%d %H:%M:%S%.f %:z")
    )?;
    writeln!(file, "sequence: {}", state.sequence_number)?;
    Ok(())
}

fn fetch_root_state(client: &Client, base_url: &str) -> Result<ReplicationState> {
    let yaml_url = format!("{}state.yaml", base_url);
    if let Some(text) = fetch_text(client, &yaml_url)? {
        return parse_state(&text)
            .with_context(|| format!("Failed to parse root state at {}", yaml_url));
    }

    let txt_url = format!("{}state.txt", base_url);
    let text = fetch_text(client, &txt_url)?
        .context("Root state not found")?;
    parse_state(&text).with_context(|| format!("Failed to parse root state at {}", txt_url))
}

fn fetch_sequence_state(
    client: &Client,
    base_url: &str,
    sequence: u64,
) -> Result<Option<ReplicationState>> {
    let url = sequence_state_url(base_url, sequence);
    let text = match fetch_text(client, &url)? {
        Some(text) => text,
        None => return Ok(None),
    };
    parse_state(&text)
        .with_context(|| format!("Failed to parse sequence state at {}", url))
        .map(Some)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    let trimmed = value.trim();
    let formats = [
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M:%S%.f %z",
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%:z",
    ];

    for format in formats {
        if let Ok(parsed) = DateTime::parse_from_str(trimmed, format) {
            return Ok(parsed.with_timezone(&Utc));
        }
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(Utc.from_utc_datetime(&parsed));
    }

    Err(anyhow::anyhow!("Unsupported timestamp format: {}", trimmed))
}

fn bootstrap_state_from_parquet(
    client: &Client,
    base_url: &str,
    data_path: &Path,
    state_path: &Path,
    delta_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(delta_dir)?;
    let stats_path = find_partition_stats_path(data_path)
        .context("partition_stats.parquet not found")?;
    let max_ts_millis = max_created_at_from_stats(&stats_path)
        .context("No max_created_at value found in partition stats")?;

    let target_ts = Utc
        .timestamp_millis_opt(max_ts_millis)
        .single()
        .context("Invalid max_created_at timestamp")?;

    let start_seq = resolve_sequence_for_timestamp(client, base_url, target_ts)?;
    let seq_state = fetch_sequence_state(client, base_url, start_seq)?;
    let timestamp = seq_state
        .as_ref()
        .map(|state| state.timestamp)
        .unwrap_or(target_ts);

    let state = ReplicationState {
        sequence_number: start_seq.saturating_sub(1),
        timestamp,
    };
    write_state(state_path, &state)?;
    log_info(&format!(
        "Bootstrapped state at sequence {} (timestamp {})",
        state.sequence_number,
        state.timestamp.to_rfc3339()
    ));
    Ok(())
}

fn find_partition_stats_path(data_path: &Path) -> Option<PathBuf> {
    let base_path = data_path.join("base/indexes/partition_stats.parquet");
    if base_path.exists() {
        return Some(base_path);
    }

    let root_path = data_path.join("indexes/partition_stats.parquet");
    if root_path.exists() {
        return Some(root_path);
    }

    None
}

fn max_created_at_from_stats(path: &Path) -> Result<i64> {
    use duckdb::Connection;

    let conn = Connection::open_in_memory()?;
    let stats_path = path.to_string_lossy().replace('\'', "''");
    let query = format!(
        "SELECT MAX(max_created_at) AS max_ts FROM read_parquet('{}')",
        stats_path
    );
    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let max_ts: Option<i64> = row.get("max_ts")?;
        return max_ts.context("max_ts is NULL");
    }
    Err(anyhow::anyhow!("partition_stats query returned no rows"))
}

fn fetch_text(client: &Client, url: &str) -> Result<Option<String>> {
    let response = client.get(url).send().with_context(|| format!("GET {}", url))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("GET {} failed", url))?;
    response.text().map(Some).context("Failed to read response body")
}

fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let response = client.get(url).send().with_context(|| format!("GET {}", url))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(anyhow::anyhow!("Remote file not found: {}", url));
    }
    let mut response = response
        .error_for_status()
        .with_context(|| format!("GET {} failed", url))?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = dest.with_extension("part");
    let mut file = File::create(&tmp_path)
        .with_context(|| format!("Failed to create {:?}", tmp_path))?;
    io::copy(&mut response, &mut file)
        .with_context(|| format!("Failed to write {}", dest.display()))?;
    fs::rename(&tmp_path, dest)
        .with_context(|| format!("Failed to rename {:?} to {:?}", tmp_path, dest))?;
    Ok(())
}

fn resolve_sequence_for_timestamp(
    client: &Client,
    base_url: &str,
    target: DateTime<Utc>,
) -> Result<u64> {
    let latest = fetch_root_state(client, base_url)?;
    if target > latest.timestamp {
        return Ok(latest.sequence_number + 1);
    }

    let mut low = 0u64;
    let mut high = latest.sequence_number;
    let mut result = latest.sequence_number;

    while low <= high {
        let mid = low + (high - low) / 2;
        let mid_state = match fetch_sequence_state(client, base_url, mid)? {
            Some(state) => state,
            None => {
                low = mid.saturating_add(1);
                continue;
            }
        };

        if mid_state.timestamp < target {
            low = mid.saturating_add(1);
        } else {
            result = mid_state.sequence_number;
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }

    Ok(result)
}

fn process_sequence(
    client: &Client,
    base_url: &str,
    sequence: u64,
    temp_root: &Path,
    delta_dir: &Path,
    args: &Args,
) -> Result<ReplicationState> {
    let seq_state = fetch_sequence_state(client, base_url, sequence)?
        .context("Missing sequence state file")?;

    let sequence_dir = temp_root.join(format!("seq-{:09}", sequence));
    if sequence_dir.exists() {
        fs::remove_dir_all(&sequence_dir)
            .with_context(|| format!("Failed to clear {:?}", sequence_dir))?;
    }
    fs::create_dir_all(&sequence_dir)?;

    let diff_path = sequence_dir.join("changeset.osm.gz");
    let diff_url = sequence_diff_url(base_url, sequence);
    download_file(client, &diff_url, &diff_path)?;

    let output_dir = sequence_dir.join("output");
    let file = File::open(&diff_path)?;
    let decoder = GzDecoder::new(file);

    let (tx, rx) = sync_channel(32);
    let continue_on_error = args.continue_on_error;
    let parser_handle = thread::spawn(move || {
        parse_xml_to_channel(decoder, tx, continue_on_error);
    });

    let schema = create_schema();
    let (total_count, summary) = consume_and_write_partitioned_parallel(
        rx,
        output_dir.clone(),
        schema,
        args.batch_size,
        None,
    )?;

    if parser_handle.join().is_err() {
        return Err(anyhow::anyhow!("Parser thread panicked"));
    }

    log_info(&format!(
        "Sequence {}: wrote {} changesets across {} partitions",
        sequence,
        total_count,
        summary.counts.len()
    ));

    move_output_files(&output_dir, delta_dir, sequence)?;

    if !args.keep_temp {
        fs::remove_dir_all(&sequence_dir)
            .with_context(|| format!("Failed to remove {:?}", sequence_dir))?;
    }

    Ok(seq_state)
}

fn move_output_files(output_dir: &Path, delta_dir: &Path, sequence: u64) -> Result<()> {
    let mut files = Vec::new();
    collect_parquet_files(output_dir, &mut files)?;

    for file_path in files {
        let (year, month) = extract_year_month(&file_path)
            .with_context(|| format!("Unable to parse year/month from {:?}", file_path))?;

        let dest_dir = delta_dir
            .join(format!("year={}", year))
            .join(format!("month={:02}", month));
        fs::create_dir_all(&dest_dir)?;

        let dest_path = dest_dir.join(format!("seq-{:09}.parquet", sequence));
        if dest_path.exists() {
            fs::remove_file(&dest_path)?;
        }

        if let Err(err) = fs::rename(&file_path, &dest_path) {
            let copy_result = fs::copy(&file_path, &dest_path);
            if copy_result.is_ok() {
                fs::remove_file(&file_path)?;
            } else {
                return Err(err.into());
            }
        }
    }

    Ok(())
}

fn collect_parquet_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
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

fn extract_year_month(path: &Path) -> Option<(i32, u32)> {
    let mut year: Option<i32> = None;
    let mut month: Option<u32> = None;

    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if let Some(value) = part.strip_prefix("year=") {
            year = value.parse::<i32>().ok();
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

fn sequence_diff_url(base_url: &str, sequence: u64) -> String {
    let (a, b, c) = sequence_path_parts(sequence);
    format!("{}{}/{}/{}.osm.gz", base_url, a, b, c)
}

fn sequence_state_url(base_url: &str, sequence: u64) -> String {
    let (a, b, c) = sequence_path_parts(sequence);
    format!("{}{}/{}/{}.state.txt", base_url, a, b, c)
}

fn sequence_path_parts(sequence: u64) -> (String, String, String) {
    let padded = format!("{:09}", sequence);
    (
        padded[0..3].to_string(),
        padded[3..6].to_string(),
        padded[6..9].to_string(),
    )
}

fn log_info(message: &str) {
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
    println!("[{}] INFO: {}", timestamp, message);
}
