//! Partition resolution and selection
//!
//! Determines which Parquet partitions to query based on date ranges.
//! Uses filesystem watching with periodic rescan fallback.

use chrono::{DateTime, Datelike, NaiveDate};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

/// How often to do a full rescan as a safety net (in seconds)
const RESCAN_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Faster rescan interval when filesystem watching is unavailable
const RESCAN_INTERVAL_FALLBACK_SECS: u64 = 30; // 30 seconds

/// Resolves partition files based on query parameters
///
/// Uses filesystem watching for instant updates when new parquet files
/// are added, with a periodic rescan fallback as a safety net.
pub struct PartitionResolver {
    root: PathBuf,
    cache: Arc<RwLock<PartitionCache>>,
    shutdown: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
}

/// Internal cache of discovered partitions
struct PartitionCache {
    partitions_by_key: HashMap<(u32, u32), Vec<PathBuf>>,
}

impl PartitionCache {
    fn new() -> Self {
        Self {
            partitions_by_key: HashMap::new(),
        }
    }

    fn scan_directories(&mut self, roots: &[PathBuf]) {
        self.partitions_by_key.clear();
        for root_dir in roots {
            let files = list_parquet_files(root_dir);
            for path in files {
                if let Some((year, month)) = parse_partition_key(&path) {
                    self.partitions_by_key
                        .entry((year, month))
                        .or_default()
                        .push(path);
                }
            }
        }
    }
}

impl PartitionResolver {
    /// Create a new partition resolver with filesystem watching
    ///
    /// # Arguments
    /// - `data_path` - Path to directory containing partitioned Parquet files
    pub fn new(data_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let root = PathBuf::from(data_path);
        let cache = Arc::new(RwLock::new(PartitionCache::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        // Determine which directories to watch
        let roots = get_data_roots(&root);

        // Initial scan
        {
            let mut cache_guard = cache.write().unwrap();
            cache_guard.scan_directories(&roots);
            let total_files: usize = cache_guard
                .partitions_by_key
                .values()
                .map(|v| v.len())
                .sum();
            tracing::info!(
                "Partition resolver initialized: {} partitions, {} files",
                cache_guard.partitions_by_key.len(),
                total_files
            );
        }

        // Set up filesystem watcher (optional - falls back to periodic rescan if it fails)
        let watcher = match setup_watcher(roots.clone(), Arc::clone(&cache)) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(
                    "Filesystem watcher unavailable ({}), relying on periodic rescan only",
                    e
                );
                None
            }
        };

        // Start periodic rescan thread (faster interval if watching unavailable)
        let rescan_interval = if watcher.is_some() {
            RESCAN_INTERVAL_SECS
        } else {
            RESCAN_INTERVAL_FALLBACK_SECS
        };
        start_rescan_thread(
            roots,
            Arc::clone(&cache),
            Arc::clone(&shutdown),
            rescan_interval,
        );

        Ok(Self {
            root,
            cache,
            shutdown,
            _watcher: watcher,
        })
    }

    /// Get partitions for a date range
    ///
    /// Maps the date range to (year, month) tuples and returns all files
    /// for those months from both base and delta directories.
    pub fn get_partitions_for_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let (start_year, start_month) = parse_date_year_month(start_date)?;
        let (end_year, end_month) = parse_date_year_month(end_date)?;

        if (start_year, start_month) > (end_year, end_month) {
            return Err("start_date is after end_date".into());
        }

        let cache = self.cache.read().unwrap();
        let mut paths = Vec::new();
        let mut year = start_year;
        let mut month = start_month;

        loop {
            if let Some(partition_paths) = cache.partitions_by_key.get(&(year, month)) {
                paths.extend(
                    partition_paths
                        .iter()
                        .map(|p| p.to_string_lossy().to_string()),
                );
            }

            if (year, month) == (end_year, end_month) {
                break;
            }

            if month == 12 {
                year += 1;
                month = 1;
            } else {
                month += 1;
            }
        }

        Ok(paths)
    }

    /// Get all available partitions sorted by (year, month)
    pub fn get_all_partitions(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let cache = self.cache.read().unwrap();
        let mut entries: Vec<_> = cache.partitions_by_key.iter().collect();
        entries.sort_by_key(|(key, _)| *key);

        let mut paths = Vec::new();
        for (_, partition_paths) in entries {
            paths.extend(
                partition_paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string()),
            );
        }

        Ok(paths)
    }

    /// Force a rescan of the filesystem
    ///
    /// Useful for testing or manual refresh
    #[allow(dead_code)]
    pub fn rescan(&self) {
        let roots = get_data_roots(&self.root);
        let mut cache = self.cache.write().unwrap();
        cache.scan_directories(&roots);
        tracing::info!("Partition resolver: manual rescan complete");
    }
}

impl Drop for PartitionResolver {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// Get the data root directories (base/ and delta/ if they exist, otherwise root)
fn get_data_roots(root: &Path) -> Vec<PathBuf> {
    let base = root.join("base");
    let delta = root.join("delta");

    if base.exists() || delta.exists() {
        let mut roots = Vec::new();
        if base.exists() {
            roots.push(base);
        }
        if delta.exists() {
            roots.push(delta);
        }
        roots
    } else {
        vec![root.to_path_buf()]
    }
}

/// Set up filesystem watcher for the given directories
///
/// Uses non-recursive watching to avoid hitting file descriptor limits.
/// Any filesystem event triggers a full rescan of the partition cache.
fn setup_watcher(
    roots: Vec<PathBuf>,
    cache: Arc<RwLock<PartitionCache>>,
) -> Result<RecommendedWatcher, Box<dyn std::error::Error>> {
    let cache_for_handler = Arc::clone(&cache);
    let roots_for_handler = roots.clone();

    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| match result {
            Ok(event) => handle_fs_event(event, &cache_for_handler, &roots_for_handler),
            Err(e) => tracing::warn!("Filesystem watch error: {}", e),
        },
        Config::default(),
    )?;

    // Watch directories non-recursively to avoid file descriptor exhaustion
    // We watch the year= directories within each root
    let mut watched = 0;
    for root in &roots {
        // Watch the root itself (catches new year= directories)
        if watcher.watch(root, RecursiveMode::NonRecursive).is_ok() {
            watched += 1;
        }

        // Watch each year= directory (catches new month= directories/files)
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("year=") {
                            if watcher.watch(&path, RecursiveMode::NonRecursive).is_ok() {
                                watched += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    if watched > 0 {
        tracing::info!("Watching {} directories for changes", watched);
        Ok(watcher)
    } else {
        Err("No directories could be watched".into())
    }
}

/// Handle a filesystem event by triggering a rescan
fn handle_fs_event(event: Event, cache: &Arc<RwLock<PartitionCache>>, roots: &[PathBuf]) {
    use notify::EventKind;

    // Only care about create/modify/remove events
    let dominated = matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    );
    if !dominated {
        return;
    }

    // Check if any path is a parquet file or a directory that might contain them
    let dominated = event.paths.iter().any(|p| {
        p.extension().and_then(|e| e.to_str()) == Some("parquet")
            || p.is_dir()
            || p.to_string_lossy().contains("year=")
            || p.to_string_lossy().contains("month=")
    });
    if !dominated {
        return;
    }

    // Do a full rescan - simpler and more reliable than tracking individual changes
    let mut cache = cache.write().unwrap();
    let old_count: usize = cache.partitions_by_key.values().map(|v| v.len()).sum();
    cache.scan_directories(roots);
    let new_count: usize = cache.partitions_by_key.values().map(|v| v.len()).sum();

    if old_count != new_count {
        tracing::info!(
            "Partition cache updated via filesystem event: {} -> {} files",
            old_count,
            new_count
        );
    }
}

/// Start a background thread for periodic rescanning
fn start_rescan_thread(
    roots: Vec<PathBuf>,
    cache: Arc<RwLock<PartitionCache>>,
    shutdown: Arc<AtomicBool>,
    interval_secs: u64,
) {
    thread::spawn(move || {
        tracing::info!(
            "Partition rescan thread started (interval: {}s)",
            interval_secs
        );
        loop {
            // Sleep in small increments to check shutdown flag
            for _ in 0..(interval_secs * 10) {
                if shutdown.load(Ordering::SeqCst) {
                    tracing::debug!("Partition rescan thread shutting down");
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }

            // Do a full rescan
            let mut cache = cache.write().unwrap();
            let old_count: usize = cache.partitions_by_key.values().map(|v| v.len()).sum();
            cache.scan_directories(&roots);
            let new_count: usize = cache.partitions_by_key.values().map(|v| v.len()).sum();

            if old_count != new_count {
                tracing::info!(
                    "Partition resolver periodic rescan: {} -> {} files",
                    old_count,
                    new_count
                );
            } else {
                tracing::debug!("Partition resolver periodic rescan: no changes");
            }
        }
    });
}

/// Recursively list all parquet files in a directory
fn list_parquet_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_parquet_files(dir, &mut files);
    files
}

/// Helper to recursively collect parquet files
fn collect_parquet_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip indexes directory
            if path.file_name().and_then(|n| n.to_str()) == Some("indexes") {
                continue;
            }
            collect_parquet_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            files.push(path);
        }
    }
}

/// Parse year and month from a partition path
///
/// Expects paths like: .../year=2024/month=05.parquet or .../year=2024/month=05/file.parquet
fn parse_partition_key(path: &Path) -> Option<(u32, u32)> {
    let mut year: Option<u32> = None;
    let mut month: Option<u32> = None;

    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if let Some(value) = part.strip_prefix("year=") {
            year = value.parse::<u32>().ok();
        } else if let Some(value) = part.strip_prefix("month=") {
            let value = value.trim_end_matches(".parquet");
            month = value.parse::<u32>().ok();
        }
    }

    match (year, month) {
        (Some(year), Some(month)) => Some((year, month)),
        _ => None,
    }
}

/// Parse a date string into (year, month)
fn parse_date_year_month(date: &str) -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let parsed_date = if let Ok(value) = NaiveDate::parse_from_str(date, "%Y%m%d") {
        value
    } else if let Ok(value) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        value
    } else if let Ok(value) = DateTime::parse_from_rfc3339(date) {
        value.naive_utc().date()
    } else {
        return Err("unsupported date format".into());
    };

    let year = u32::try_from(parsed_date.year()).map_err(|_| "unsupported year for partition")?;
    Ok((year, parsed_date.month()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_partitions() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let year_2023 = dir.path().join("year=2023");
        let year_2024 = dir.path().join("year=2024");
        fs::create_dir_all(&year_2023).expect("year dir");
        fs::create_dir_all(&year_2024).expect("year dir");

        fs::write(year_2023.join("month=12.parquet"), b"data").expect("write 2023-12");
        fs::write(year_2024.join("month=05.parquet"), b"data").expect("write 2024-05");
        fs::write(year_2024.join("month=06.parquet"), b"data").expect("write 2024-06");

        dir
    }

    #[test]
    fn test_get_all_partitions_sorted() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver.get_all_partitions().expect("partitions");
        assert_eq!(partitions.len(), 3);
        assert!(partitions[0].contains("year=2023"));
        assert!(partitions[1].contains("year=2024"));
    }

    #[test]
    fn test_get_partitions_for_date_range() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");
        let partitions = resolver
            .get_partitions_for_date_range("20240501", "20240630")
            .expect("range");
        assert_eq!(partitions.len(), 2);
        assert!(partitions[0].contains("month=05"));
        assert!(partitions[1].contains("month=06"));
    }

    #[test]
    fn test_dynamic_file_addition() {
        let dir = setup_partitions();
        let resolver = PartitionResolver::new(dir.path().to_str().unwrap()).expect("resolver");

        // Initially 3 files
        let partitions = resolver.get_all_partitions().expect("partitions");
        assert_eq!(partitions.len(), 3);

        // Add a new file
        let year_2024 = dir.path().join("year=2024");
        fs::write(year_2024.join("month=07.parquet"), b"data").expect("write 2024-07");

        // Give the watcher time to process
        thread::sleep(Duration::from_millis(500));

        // Should now have 4 files
        let partitions = resolver.get_all_partitions().expect("partitions");
        assert_eq!(partitions.len(), 4);
    }
}
