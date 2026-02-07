use anyhow::{Context, Result};
use ch_ch_changes_core::{
    metadata::{update_metadata_incremental, BatchStats},
    parsing::parse_xml_to_channel,
    schema::create_schema,
    writer::consume_and_write_partitioned_parallel,
    Changeset,
};
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use clap::Parser;
use duckdb::Connection;
use flate2::read::GzDecoder;
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

/// Retry network operations with exponential backoff
fn retry_network<F, T>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    const MAX_RETRIES: u32 = 5;
    const INITIAL_BACKOFF: Duration = Duration::from_secs(2);

    let mut attempt = 0;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        match operation() {
            Ok(result) => return Ok(result),
            Err(err) => {
                // Check if error is retryable
                let is_retryable = if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
                    // Retry on network errors (timeouts, connection failures) and server errors (5xx)
                    if reqwest_err.is_timeout()
                        || reqwest_err.is_connect()
                        || reqwest_err.is_request()
                        || reqwest_err.is_body()
                    {
                        true
                    } else if let Some(status) = reqwest_err.status() {
                        // Retry on server errors (5xx) and 429 (rate limit)
                        status.is_server_error() || status.as_u16() == 429
                    } else {
                        // Other reqwest errors - be permissive and retry
                        true
                    }
                } else {
                    // Non-reqwest errors (IO, parsing, etc.) - don't retry
                    false
                };

                if !is_retryable || attempt >= MAX_RETRIES {
                    return Err(err);
                }

                attempt += 1;
                log_info(&format!(
                    "Network error (attempt {}/{}): {}. Retrying in {:?}...",
                    attempt, MAX_RETRIES, err, backoff
                ));
                std::thread::sleep(backoff);
                backoff *= 2; // Exponential backoff
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Root data path (contains base/ and delta/ or a flat layout)
    #[arg(
        long,
        env = "DATA_PATH",
        default_value = "/opt/changeset-data/changesets"
    )]
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

    /// Disable automatic state bootstrapping from existing parquet files if no state file exists
    #[arg(long, default_value_t = false)]
    no_bootstrap: bool,

    /// Batch size for writing records
    #[arg(long, default_value_t = 10000, hide = true)]
    batch_size: usize,

    /// Continue processing on parse errors (saves what was successfully parsed)
    #[arg(long, default_value_t = false)]
    continue_on_error: bool,

    /// Keep temporary files for debugging
    #[arg(long, default_value_t = false)]
    keep_temp: bool,

    /// Batch sequences by hour (recommended for production)
    #[arg(long, default_value_t = true)]
    hourly_batching: bool,

    /// Maximum sequences to buffer before forcing a write (safety limit)
    #[arg(long, default_value_t = 120)]
    max_batch_sequences: u64,

    /// Enable automatic compaction of old delta files
    #[arg(long, env = "ENABLE_COMPACTION", default_value_t = false)]
    enable_compaction: bool,

    /// Hours to wait before compacting a delta file (default: 24 = compact yesterday's data)
    #[arg(long, env = "COMPACTION_AGE_HOURS", default_value_t = 24)]
    compaction_age_hours: u64,
}

#[derive(Debug, Clone)]
struct ReplicationState {
    sequence_number: u64,
    timestamp: DateTime<Utc>,
}

/// Accumulates changesets across multiple sequences into hourly batches
struct BatchAccumulator {
    changesets: Vec<Changeset>,
    batch_hour: Option<DateTime<Utc>>,
    start_sequence: u64,
    end_sequence: u64,
    sequence_count: u64,
}

impl BatchAccumulator {
    fn new(start_sequence: u64) -> Self {
        Self {
            changesets: Vec::new(),
            batch_hour: None,
            start_sequence,
            end_sequence: start_sequence,
            sequence_count: 0,
        }
    }

    /// Add changesets from a sequence. Returns true if we've crossed an hour boundary.
    fn add(
        &mut self,
        mut changesets: Vec<Changeset>,
        sequence: u64,
        sequence_timestamp: DateTime<Utc>,
    ) -> bool {
        let seq_hour = sequence_timestamp
            .date_naive()
            .and_hms_opt(sequence_timestamp.hour(), 0, 0)
            .map(|naive| Utc.from_utc_datetime(&naive));

        // Initialize batch_hour on first add
        if self.batch_hour.is_none() {
            self.batch_hour = seq_hour;
        }

        // Check if we've crossed an hour boundary
        let crossed_hour = seq_hour != self.batch_hour;

        if !crossed_hour {
            // Set replication_sequence for all changesets in this batch
            for cs in &mut changesets {
                cs.replication_sequence = Some(sequence);
            }
            self.changesets.extend(changesets);
            self.end_sequence = sequence;
            self.sequence_count += 1;
        }

        crossed_hour
    }

    /// Get the current batch size
    fn len(&self) -> usize {
        self.changesets.len()
    }

    /// Get the batch hour as YYYYMMDDHH string
    fn hour_string(&self) -> Option<String> {
        self.batch_hour.map(|dt| dt.format("%Y%m%d%H").to_string())
    }

    /// Take all accumulated changesets, resetting the accumulator for the next batch
    fn take_batch(&mut self, new_hour: DateTime<Utc>, new_sequence: u64) -> Vec<Changeset> {
        let batch = std::mem::take(&mut self.changesets);
        self.batch_hour = Some(new_hour);
        self.start_sequence = new_sequence;
        self.end_sequence = new_sequence;
        self.sequence_count = 0;
        batch
    }

    /// Flush remaining changesets (for end of catch-up)
    fn flush(&mut self) -> Vec<Changeset> {
        std::mem::take(&mut self.changesets)
    }
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

    let mut last_state = load_local_state(&state_path)?;
    if last_state.is_none() {
        last_state = Some(initialize_state(&client, &base_url, &args, &data_path)?);
    }
    let mut last_state = last_state.expect("state must be initialized");

    loop {
        let remote_state = fetch_root_state(&client, &base_url)?;

        // OSM changeset replication state files are off-by-one.
        let mut end_sequence = remote_state.sequence_number + 1;

        if last_state.sequence_number >= end_sequence {
            if args.once {
                break;
            }
            sleep_poll(args.poll_interval_seconds);
            continue;
        }

        if let Some(max_per_run) = args.max_sequences_per_run {
            end_sequence = end_sequence.min(last_state.sequence_number + max_per_run);
        }

        if args.hourly_batching {
            // Hourly batching mode: accumulate changesets across sequences
            let mut accumulator = BatchAccumulator::new(last_state.sequence_number + 1);
            let mut last_seq_state_in_batch: Option<ReplicationState> = None;

            for seq in (last_state.sequence_number + 1)..=end_sequence {
                log_info(&format!("Fetching sequence {}", seq));

                let seq_state = match fetch_sequence_state(&client, &base_url, seq)? {
                    Some(s) => s,
                    None => {
                        log_info(&format!(
                            "Sequence {} not found (yet), stopping batch.",
                            seq
                        ));
                        break;
                    }
                };

                let mut changesets = parse_sequence(&client, &base_url, seq, &temp_root, &args)?;

                // Try to add to accumulator - returns true if hour boundary crossed
                let crossed_hour = accumulator.add(changesets.clone(), seq, seq_state.timestamp);

                if crossed_hour {
                    // Write the accumulated batch before the boundary
                    let hour_str = accumulator
                        .hour_string()
                        .context("Missing hour string for batch flush")?;
                    let state_to_flush = last_seq_state_in_batch
                        .take()
                        .context("Missing last sequence state for batch flush")?;

                    log_info(&format!(
                        "Hour boundary crossed at sequence {} - writing batch for {}",
                        seq, hour_str
                    ));
                    let batch = accumulator.take_batch(
                        seq_state
                            .timestamp
                            .date_naive()
                            .and_hms_opt(seq_state.timestamp.hour(), 0, 0)
                            .map(|naive| Utc.from_utc_datetime(&naive))
                            .unwrap(),
                        seq,
                    );
                    write_hourly_batch(
                        batch,
                        &delta_dir,
                        &hour_str,
                        &temp_root,
                        args.batch_size,
                        args.keep_temp,
                    )?;
                    write_state(&state_path, &state_to_flush)?;
                    last_state = state_to_flush;

                    // Run compaction if enabled (non-blocking, errors are logged but not fatal)
                    if args.enable_compaction {
                        let _ = maybe_run_compaction(&delta_dir, args.compaction_age_hours);
                    }

                    // Now add the current sequence to the new batch
                    for cs in &mut changesets {
                        cs.replication_sequence = Some(seq);
                    }
                    accumulator.add(changesets, seq, seq_state.timestamp);
                }

                last_seq_state_in_batch = Some(seq_state.clone());

                // Safety check: force flush if we've accumulated too many sequences
                if accumulator.sequence_count >= args.max_batch_sequences {
                    let hour_str = accumulator
                        .hour_string()
                        .context("Missing hour string for max-sequence flush")?;
                    let state_to_flush = last_seq_state_in_batch
                        .take()
                        .context("Missing last sequence state for max-sequence flush")?;

                    log_info(&format!(
                        "Max batch sequences ({}) reached - flushing batch for {}",
                        args.max_batch_sequences, hour_str
                    ));
                    let batch = accumulator.flush();
                    write_hourly_batch(
                        batch,
                        &delta_dir,
                        &hour_str,
                        &temp_root,
                        args.batch_size,
                        args.keep_temp,
                    )?;
                    write_state(&state_path, &state_to_flush)?;
                    last_state = state_to_flush;
                    accumulator = BatchAccumulator::new(seq + 1);
                }
            }

            // Flush any remaining changesets at end of catch-up
            if last_seq_state_in_batch.is_some() && accumulator.len() > 0 {
                let hour_str = accumulator
                    .hour_string()
                    .context("Missing hour string for final flush")?;
                let state_to_flush = last_seq_state_in_batch
                    .take()
                    .context("Missing last sequence state for final flush")?;
                log_info(&format!(
                    "End of catch-up - flushing final batch for {} ({} changesets)",
                    hour_str,
                    accumulator.len()
                ));
                let batch = accumulator.flush();
                write_hourly_batch(
                    batch,
                    &delta_dir,
                    &hour_str,
                    &temp_root,
                    args.batch_size,
                    args.keep_temp,
                )?;
                write_state(&state_path, &state_to_flush)?;
                last_state = state_to_flush;
            }
        } else {
            // Original per-sequence mode
            for seq in (last_state.sequence_number + 1)..=end_sequence {
                log_info(&format!("Processing sequence {}", seq));

                // Check existence first since we might have overshot by one
                if fetch_sequence_state(&client, &base_url, seq)?.is_none() {
                    log_info(&format!("Sequence {} not found (yet), stopping.", seq));
                    break;
                }

                let seq_state =
                    process_sequence(&client, &base_url, seq, &temp_root, &delta_dir, &args)?;
                write_state(&state_path, &seq_state)?;
                last_state = seq_state;
            }
        }

        if args.once {
            break;
        }
    }

    Ok(())
}

fn build_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(2)
        .tcp_keepalive(Duration::from_secs(60))
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

fn initialize_state(
    client: &Client,
    base_url: &str,
    args: &Args,
    data_path: &Path,
) -> Result<ReplicationState> {
    if let Some(seq) = args.start_sequence {
        let state =
            fetch_sequence_state(client, base_url, seq)?.unwrap_or_else(|| ReplicationState {
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
        let state = fetch_sequence_state(client, base_url, seq)?.unwrap_or(ReplicationState {
            sequence_number: seq,
            timestamp: target,
        });
        return Ok(ReplicationState {
            sequence_number: seq.saturating_sub(1),
            timestamp: state.timestamp,
        });
    }

    if !args.no_bootstrap {
        if let Some(state) = try_recover_state_from_parquet(client, base_url, data_path)? {
            log_info(&format!(
                "Bootstrapped state at sequence {} (timestamp {})",
                state.sequence_number,
                state.timestamp.to_rfc3339()
            ));
            return Ok(state);
        }
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
    let state =
        parse_state(&text).with_context(|| format!("Failed to parse state file at {:?}", path))?;
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
    let mut file =
        File::create(path).with_context(|| format!("Failed to write state file at {:?}", path))?;
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
    let text = fetch_text(client, &txt_url)?.context("Root state not found")?;
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
    let mut state =
        parse_state(&text).with_context(|| format!("Failed to parse sequence state at {}", url))?;

    if state.sequence_number != sequence {
        // OSM changeset replication state files point to the *previous* sequence number (N-1)
        // while containing the timestamp for the *current* sequence (N).
        // We accept N-1 as valid but update it to N for our internal tracking.
        if state.sequence_number == sequence.saturating_sub(1) {
            state.sequence_number = sequence;
        } else {
            log_warn(&format!(
                "State file for sequence {} claims to be sequence {}. Forcing sequence {}.",
                sequence, state.sequence_number, sequence
            ));
            state.sequence_number = sequence;
        }
    }

    Ok(Some(state))
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

fn try_recover_state_from_parquet(
    client: &Client,
    base_url: &str,
    data_path: &Path,
) -> Result<Option<ReplicationState>> {
    let mut files = Vec::new();

    // Check for legacy base/ directory structure
    let base_path = data_path.join("base");
    if base_path.exists() {
        collect_parquet_files(&base_path, &mut files)?;
    }

    // Check for delta/ directory
    let delta_path = data_path.join("delta");
    if delta_path.exists() {
        collect_parquet_files(&delta_path, &mut files)?;
    }

    // If no files found in subdirectories, check the root data_path itself
    // This handles flat Hive-partitioned layouts (year=YYYY/month=MM.parquet)
    if files.is_empty() && data_path.exists() {
        collect_parquet_files(data_path, &mut files)?;
    }

    if files.is_empty() {
        return Ok(None);
    }

    let max_ts_millis = max_created_at_from_parquet_files(&files)
        .context("No created_at value found in parquet data")?;

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
    Ok(Some(state))
}

fn max_created_at_from_parquet_files(files: &[PathBuf]) -> Result<i64> {
    let conn = Connection::open_in_memory()?;
    let file_list: Vec<String> = files
        .iter()
        .map(|path| escape_sql_string(path.as_path()))
        .collect();
    let list_sql = file_list.join("','");
    let query = format!(
        "SELECT MAX(epoch_ms(created_at)) AS max_ts FROM read_parquet(['{}'])",
        list_sql
    );
    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let max_ts: Option<i64> = row.get("max_ts")?;
        return max_ts.context("max_ts is NULL");
    }
    Err(anyhow::anyhow!("created_at query returned no rows"))
}

/// Fetch HTTP response with retry for network errors and server errors (5xx)
/// Returns Ok(None) for 404 Not Found, Ok(Some(Response)) for success (2xx),
/// Err on other client errors (4xx) or other failures
fn fetch_response_allow_404(client: &Client, url: &str) -> Result<Option<Response>> {
    retry_network(|| {
        let response = client
            .get(url)
            .send()
            .with_context(|| format!("GET {}", url))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .with_context(|| format!("GET {} failed", url))?;
        Ok(Some(response))
    })
}

fn fetch_text(client: &Client, url: &str) -> Result<Option<String>> {
    match fetch_response_allow_404(client, url)? {
        Some(response) => response
            .text()
            .map(Some)
            .context("Failed to read response body"),
        None => Ok(None),
    }
}

fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let mut response = retry_network(|| {
        let response = client
            .get(url)
            .send()
            .with_context(|| format!("GET {}", url))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(anyhow::anyhow!("Remote file not found: {}", url));
        }
        response
            .error_for_status()
            .with_context(|| format!("GET {} failed", url))
    })?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = dest.with_extension("part");
    let mut file =
        File::create(&tmp_path).with_context(|| format!("Failed to create {:?}", tmp_path))?;
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
    let seq_state =
        fetch_sequence_state(client, base_url, sequence)?.context("Missing sequence state file")?;

    // Parse sequence to get changesets for metadata computation
    let changesets = parse_sequence(client, base_url, sequence, temp_root, args)?;
    let batch_stats = BatchStats::from_changesets(&changesets);

    let sequence_dir = temp_root.join(format!("seq-{:09}", sequence));
    if sequence_dir.exists() {
        fs::remove_dir_all(&sequence_dir)
            .with_context(|| format!("Failed to clear {:?}", sequence_dir))?;
    }
    fs::create_dir_all(&sequence_dir)?;

    let output_dir = sequence_dir.join("output");
    fs::create_dir_all(&output_dir)?;

    // Write changesets
    let (tx, rx) = sync_channel(32);
    let output_dir_clone = output_dir.clone();
    let batch_size = args.batch_size;
    let writer_handle = thread::spawn(move || {
        let schema = create_schema();
        consume_and_write_partitioned_parallel(rx, output_dir_clone, schema, batch_size, None)
    });

    // Send changesets in batches
    let chunk_size = 1000;
    for chunk in changesets.chunks(chunk_size) {
        tx.send(Ok(chunk.to_vec()))?;
    }
    drop(tx);

    let (total_count, summary) = writer_handle
        .join()
        .map_err(|_| anyhow::anyhow!("Writer thread panicked"))??;

    log_info(&format!(
        "Sequence {}: wrote {} changesets across {} partitions",
        sequence,
        total_count,
        summary.counts.len()
    ));

    move_output_files(&output_dir, delta_dir, sequence)?;

    // Update metadata.json incrementally
    let metadata_path = delta_dir.join("metadata.json");
    if let Err(e) = update_metadata_incremental(&metadata_path, &batch_stats) {
        log_warn(&format!("Failed to update metadata.json: {}", e));
    }

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

fn log_warn(message: &str) {
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
    println!("[{}] WARN: {}", timestamp, message);
}

/// Parse a sequence and return the changesets without writing to disk
fn parse_sequence(
    client: &Client,
    base_url: &str,
    sequence: u64,
    temp_root: &Path,
    args: &Args,
) -> Result<Vec<Changeset>> {
    let sequence_dir = temp_root.join(format!("seq-{:09}", sequence));
    if sequence_dir.exists() {
        fs::remove_dir_all(&sequence_dir)
            .with_context(|| format!("Failed to clear {:?}", sequence_dir))?;
    }
    fs::create_dir_all(&sequence_dir)?;

    let diff_path = sequence_dir.join("changeset.osm.gz");
    let diff_url = sequence_diff_url(base_url, sequence);
    download_file(client, &diff_url, &diff_path)?;

    let file = File::open(&diff_path)?;
    let decoder = GzDecoder::new(file);

    let (tx, rx) = sync_channel(32);
    let continue_on_error = args.continue_on_error;
    let parser_handle = thread::spawn(move || {
        parse_xml_to_channel(decoder, tx, continue_on_error);
    });

    // Collect all changesets
    let mut all_changesets = Vec::new();
    for batch_result in rx {
        let batch = batch_result?;
        all_changesets.extend(batch);
    }

    if parser_handle.join().is_err() {
        return Err(anyhow::anyhow!("Parser thread panicked"));
    }

    // Clean up temp directory
    if !args.keep_temp {
        fs::remove_dir_all(&sequence_dir)
            .with_context(|| format!("Failed to remove {:?}", sequence_dir))?;
    }

    Ok(all_changesets)
}

/// Write a batch of changesets to hourly-partitioned parquet files
fn write_hourly_batch(
    changesets: Vec<Changeset>,
    delta_dir: &Path,
    hour_string: &str,
    temp_root: &Path,
    batch_size: usize,
    keep_temp: bool,
) -> Result<()> {
    if changesets.is_empty() {
        return Ok(());
    }

    // Compute batch statistics for metadata update
    let batch_stats = BatchStats::from_changesets(&changesets);

    let batch_dir = temp_root.join(format!("batch-{}", hour_string));
    if batch_dir.exists() {
        fs::remove_dir_all(&batch_dir)?;
    }
    fs::create_dir_all(&batch_dir)?;

    let output_dir = batch_dir.join("output");
    fs::create_dir_all(&output_dir)?;

    // Write changesets using partitioned writer
    let (tx, rx) = sync_channel(32);
    let output_dir_clone = output_dir.clone();
    let writer_handle = thread::spawn(move || {
        let schema = create_schema();
        consume_and_write_partitioned_parallel(rx, output_dir_clone, schema, batch_size, None)
    });

    // Send changesets in batches
    let chunk_size = 1000;
    for chunk in changesets.chunks(chunk_size) {
        tx.send(Ok(chunk.to_vec()))?;
    }
    drop(tx);

    let (total_count, summary) = writer_handle
        .join()
        .map_err(|_| anyhow::anyhow!("Writer thread panicked"))??;

    log_info(&format!(
        "Batch {}: wrote {} changesets across {} partitions",
        hour_string,
        total_count,
        summary.counts.len()
    ));

    // Move output files with hourly naming
    move_output_files_hourly(&output_dir, delta_dir, hour_string)?;

    // Update metadata.json incrementally
    let metadata_path = delta_dir.join("metadata.json");
    if let Err(e) = update_metadata_incremental(&metadata_path, &batch_stats) {
        log_warn(&format!("Failed to update metadata.json: {}", e));
    }

    if !keep_temp {
        fs::remove_dir_all(&batch_dir)
            .with_context(|| format!("Failed to remove {:?}", batch_dir))?;
    }

    Ok(())
}

/// Move parquet files to delta directory with YYYYMMDDHH naming
fn move_output_files_hourly(output_dir: &Path, delta_dir: &Path, hour_string: &str) -> Result<()> {
    let mut files = Vec::new();
    collect_parquet_files(output_dir, &mut files)?;

    for file_path in files {
        let (year, month) = extract_year_month(&file_path)
            .with_context(|| format!("Unable to parse year/month from {:?}", file_path))?;

        let dest_dir = delta_dir
            .join(format!("year={}", year))
            .join(format!("month={:02}", month));
        fs::create_dir_all(&dest_dir)?;

        let dest_path = dest_dir.join(format!("{}.parquet", hour_string));
        if dest_path.exists() {
            merge_parquet_files(&dest_path, &file_path)?;
            continue;
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

fn merge_parquet_files(existing_path: &Path, new_path: &Path) -> Result<()> {
    let temp_path = existing_path.with_extension("parquet.tmp");
    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
    }

    let existing_sql = escape_sql_string(existing_path);
    let new_sql = escape_sql_string(new_path);
    let temp_sql = escape_sql_string(&temp_path);
    let sql = format!(
        "COPY (SELECT * FROM read_parquet('{}') UNION ALL SELECT * FROM read_parquet('{}')) TO '{}' (FORMAT 'parquet')",
        existing_sql, new_sql, temp_sql
    );

    let conn = Connection::open_in_memory()?;
    conn.execute(&sql, [])?;

    fs::remove_file(existing_path)?;
    fs::rename(&temp_path, existing_path)?;
    fs::remove_file(new_path)?;

    Ok(())
}

fn escape_sql_string(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

/// Compact old delta files in a month directory into a single file.
/// Only compacts files older than `min_age` to avoid interfering with active writes.
/// Returns the number of files compacted (0 if nothing to do).
fn compact_month_deltas(
    delta_dir: &Path,
    year: i32,
    month: u32,
    min_age: Duration,
) -> Result<usize> {
    let month_dir = delta_dir
        .join(format!("year={}", year))
        .join(format!("month={:02}", month));

    if !month_dir.exists() {
        return Ok(0);
    }

    // Find all parquet files in the month directory
    let mut files_to_compact: Vec<PathBuf> = Vec::new();
    let now = std::time::SystemTime::now();

    for entry in fs::read_dir(&month_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("parquet") {
            continue;
        }

        // Skip files that are too new
        if let Ok(metadata) = path.metadata() {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age < min_age {
                        continue;
                    }
                }
            }
        }

        // Skip the compacted file itself
        if path.file_name().and_then(|n| n.to_str()) == Some("compacted.parquet") {
            continue;
        }

        files_to_compact.push(path);
    }

    // Need at least 2 files to make compaction worthwhile
    if files_to_compact.len() < 2 {
        return Ok(0);
    }

    log_info(&format!(
        "Compacting {} delta files for {}-{:02}",
        files_to_compact.len(),
        year,
        month
    ));

    // Build the compaction query with deduplication
    let file_list: Vec<String> = files_to_compact
        .iter()
        .map(|p| format!("'{}'", escape_sql_string(p)))
        .collect();

    let compacted_path = month_dir.join("compacted.parquet");
    let temp_path = month_dir.join("compacted.parquet.tmp");

    // Include existing compacted file if present
    let source = if compacted_path.exists() {
        let mut all_files = file_list.clone();
        all_files.push(format!("'{}'", escape_sql_string(&compacted_path)));
        format!("[{}]", all_files.join(","))
    } else {
        format!("[{}]", file_list.join(","))
    };

    let temp_sql = escape_sql_string(&temp_path);
    let sql = format!(
        "COPY (
            SELECT * FROM read_parquet({}, union_by_name=true, hive_partitioning=false)
            QUALIFY ROW_NUMBER() OVER (PARTITION BY id ORDER BY replication_sequence DESC NULLS LAST) = 1
        ) TO '{}' (FORMAT PARQUET, COMPRESSION SNAPPY)",
        source, temp_sql
    );

    let conn = Connection::open_in_memory()?;
    conn.execute(&sql, [])?;

    // Atomic swap
    if compacted_path.exists() {
        fs::remove_file(&compacted_path)?;
    }
    fs::rename(&temp_path, &compacted_path)?;

    // Remove the old individual files
    let compacted_count = files_to_compact.len();
    for file in files_to_compact {
        if let Err(e) = fs::remove_file(&file) {
            log_info(&format!("Warning: failed to remove {}: {}", file.display(), e));
        }
    }

    log_info(&format!(
        "Compaction complete for {}-{:02}: {} files merged into compacted.parquet",
        year, month, compacted_count
    ));

    Ok(compacted_count)
}

/// Run compaction for hours that are old enough.
/// Called after flushing an hourly batch.
fn maybe_run_compaction(delta_dir: &Path, compaction_age_hours: u64) -> Result<()> {
    let min_age = Duration::from_secs(compaction_age_hours * 3600);
    let now = Utc::now();

    // Compact the previous month if we're past the age threshold
    // This is conservative - only compact months that are definitely "done"
    let target_time = now - chrono::Duration::hours(compaction_age_hours as i64);
    let year = target_time.year();
    let month = target_time.month();

    match compact_month_deltas(delta_dir, year, month, min_age) {
        Ok(count) if count > 0 => {
            log_info(&format!("Compaction completed: {} files processed", count));
        }
        Ok(_) => {} // Nothing to compact
        Err(e) => {
            log_info(&format!("Compaction warning (non-fatal): {}", e));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use duckdb::Connection;
    use tempfile::TempDir;

    fn write_test_parquet(path: &Path, rows: &[(i64, i64, &str)]) -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE changesets (
                id BIGINT,
                replication_sequence BIGINT,
                created_at TIMESTAMP,
                closed_at TIMESTAMP,
                open BOOLEAN,
                user VARCHAR,
                uid BIGINT,
                min_lat DOUBLE,
                min_lon DOUBLE,
                max_lat DOUBLE,
                max_lon DOUBLE,
                num_changes INTEGER,
                comments_count INTEGER,
                description VARCHAR,
                tags MAP(VARCHAR, VARCHAR)
            );",
        )?;

        for (id, seq, created_at) in rows {
            let sql = format!(
                "INSERT INTO changesets VALUES
                    ({}, {}, TIMESTAMP '{}', NULL, false, 'alice', 10, 1.0, 2.0, 3.0, 4.0, 5, 6, 'first', MAP {{'comment': 'test'}});",
                id, seq, created_at
            );
            conn.execute_batch(&sql)?;
        }

        let path_sql = escape_sql_string(path);
        let copy_sql = format!("COPY changesets TO '{}' (FORMAT PARQUET);", path_sql);
        conn.execute_batch(&copy_sql)?;
        Ok(())
    }

    #[test]
    fn test_batch_accumulator_hour_boundary() {
        let start_seq = 10;
        let mut acc = BatchAccumulator::new(start_seq);
        let ts1 = Utc.with_ymd_and_hms(2024, 5, 1, 10, 0, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 5, 1, 11, 0, 0).unwrap();

        let mut changesets = Vec::new();
        for id in 1..=3 {
            changesets.push(Changeset {
                id,
                replication_sequence: None,
                created_at: None,
                closed_at: None,
                open: false,
                user: None,
                uid: None,
                min_lat: None,
                min_lon: None,
                max_lat: None,
                max_lon: None,
                num_changes: 0,
                comments_count: 0,
                description: None,
                tags: std::collections::HashMap::new(),
            });
        }

        let crossed = acc.add(changesets.clone(), 100, ts1);
        assert!(!crossed);
        assert_eq!(acc.sequence_count, 1);
        assert_eq!(acc.len(), 3);
        assert!(acc
            .changesets
            .iter()
            .all(|cs| cs.replication_sequence == Some(100)));

        let crossed = acc.add(changesets, 101, ts2);
        assert!(crossed);
        assert_eq!(acc.sequence_count, 1);
        assert_eq!(acc.len(), 3);
        assert_eq!(acc.hour_string().as_deref(), Some("2024050110"));
    }

    #[test]
    fn test_merge_parquet_files() {
        let dir = TempDir::new().expect("tempdir");
        let existing = dir.path().join("existing.parquet");
        let incoming = dir.path().join("incoming.parquet");

        write_test_parquet(&existing, &[(1, 100, "2024-05-01 00:00:00")]).expect("write existing");
        write_test_parquet(&incoming, &[(2, 110, "2024-05-01 01:00:00")]).expect("write incoming");

        merge_parquet_files(&existing, &incoming).expect("merge");
        assert!(existing.exists());
        assert!(!incoming.exists());

        let conn = Connection::open_in_memory().expect("duckdb");
        let path_sql = escape_sql_string(&existing);
        let mut stmt = conn
            .prepare(&format!(
                "SELECT COUNT(*) AS cnt FROM read_parquet('{}')",
                path_sql
            ))
            .expect("prepare");
        let mut rows = stmt.query([]).expect("query");
        let row = rows.next().expect("row").expect("row result");
        let count: i64 = row.get("cnt").expect("count");
        assert_eq!(count, 2);
    }

    #[test]
    fn test_max_created_at_from_parquet_files() {
        let dir = TempDir::new().expect("tempdir");
        let first = dir.path().join("first.parquet");
        let second = dir.path().join("second.parquet");

        write_test_parquet(&first, &[(1, 100, "2024-05-01 00:00:00")]).expect("write first");
        write_test_parquet(&second, &[(2, 110, "2024-05-02 12:00:00")]).expect("write second");

        let files = vec![first, second];
        let max_ts = max_created_at_from_parquet_files(&files).expect("max ts");
        let expected = Utc
            .with_ymd_and_hms(2024, 5, 2, 12, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(max_ts, expected);
    }
}
