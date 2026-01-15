# Memory Leak Analysis - Changesets to Parquet

## Summary
The 54GB memory footprint is caused by **unbounded HashMap accumulation** in the IndexCollector, combined with worker thread memory multiplication. The primary culprit is the `user_stats` HashMap which grows with every unique OSM user in the dataset.

## Environment
- Input: 7.5GB bzip2 file (changesets-251117.osm.bz2)
- Configuration: 8 workers, partitioned output, index generation enabled
- Estimated data: ~150-200M changesets, ~10-30M unique users, ~252 month partitions

## Root Causes (in order of severity)

### 1. **PRIMARY CULPRIT: Unbounded user_stats HashMap**
**Location**: `src/main.rs:481` (in uncommitted changes)

```rust
struct IndexCollector {
    user_stats: HashMap<i64, UserStats>,  // ← UNBOUNDED GROWTH
    ...
}
```

**Impact**:
- Grows with every unique user ID in the entire OSM history
- With 10-30 million unique users × ~120 bytes per UserStats entry
- **Estimated memory: 1.2 - 3.6 GB for user_stats alone**
- Never flushed to disk until the very end
- This is the main memory leak

**Why it happens**:
- Line 1007-1009 in current code: Every changeset updates this HashMap
- The HashMap persists for the entire processing run
- UserStats stores: uid (8 bytes), username (String ~20 bytes), counts and timestamps (~32 bytes), HashMap overhead (~60 bytes) = ~120 bytes per user

### 2. **SECONDARY: PartitionedWriter HashMaps (per worker)**
**Location**: `src/main.rs:251-262`

```rust
struct PartitionedWriter {
    writers: HashMap<PartitionKey, ArrowWriter<File>>,      // File handles
    builders: HashMap<PartitionKey, BatchBuilders>,         // Arrow builders
    partition_counts: HashMap<PartitionKey, usize>,
    partition_stats: HashMap<PartitionKey, PartitionStats>,
}
```

**Impact**:
- Each worker maintains ALL partition builders simultaneously
- ~252 partitions (year/month from 2005-2024)
- 8 workers × 252 partitions = 2,016 BatchBuilders instances
- BatchBuilders with 4096 capacity (uncommitted) ≈ 50-100 KB each
- **Estimated memory: 8 workers × 252 partitions × 80KB ≈ 150-200 MB**
- Plus file handles: 8 × 252 = 2,016 open files

**Why it happens**:
- Round-robin distribution means every worker sees all time periods
- Each worker creates a builder for every partition it encounters
- No mechanism to flush/close old partitions

### 3. **PARTIALLY FIXED: open_changesets accumulation**
**Status**: Fixed in uncommitted changes ✓

The uncommitted changes already address this by:
- Streaming to disk every 10K changesets (INDEX_FLUSH_BATCH_SIZE)
- Changed from `Vec<Changeset>` to streaming `ArrowWriter`

### 4. **Channel buffering (minor)**
**Location**: `src/main.rs:21, 25, 963, 1138`

**Impact**:
- Parser → dispatcher: sync_channel(32) × 1000 changesets = 32K changesets
- 8 workers: 8 × 32 × 1000 = 256K changesets
- Total ~288K changesets in flight ≈ 100-150 MB
- This is acceptable and provides good throughput

## Memory Growth Timeline

1. **First 10 minutes**: 500 MB
   - Channel buffers fill: ~100 MB
   - Partition builders grow: ~150 MB
   - user_stats starts growing: ~100 MB (initial users)
   - open_changesets streaming works well

2. **After 1 hour**: 3-5 GB
   - user_stats: ~1-2 GB (millions of users encountered)
   - Partition builders: ~200 MB (all partitions seen)
   - Rest stable

3. **After 8+ hours** (overnight): 10-54 GB
   - **user_stats: 3-5+ GB** (all unique users)
   - Partition builders: ~200 MB
   - **Additional leak from RSS memory fragmentation**
   - iTerm rendering all the status updates might also contribute

## Why This Wasn't a Problem Before Multithreading

The original single-threaded version:
1. Had the same user_stats issue (would still grow to 3-5 GB)
2. But only 1 set of partition builders (not 8×)
3. Less memory fragmentation
4. Likely crashed or ran slower, but you may not have run it on full dataset

## Solutions (in priority order)

### Solution 1: Stream user_stats to disk periodically (RECOMMENDED)
**Complexity**: Medium | **Impact**: Eliminates main leak

Similar to what was done for open_changesets:
1. Write user_stats to disk periodically (e.g., every 100K unique users)
2. At the end, read all partial files and merge
3. This keeps user_stats memory bounded to ~10-20 MB

**Implementation**:
- Add `flush_user_stats()` method
- Track count of users in memory
- Flush to `indexes/user_stats_partial_001.parquet` etc.
- Merge at the end using Arrow IPC or DuckDB

### Solution 2: Reduce worker count (QUICK FIX)
**Complexity**: None | **Impact**: Reduces worker memory by 8×

```bash
# Try with 2 workers instead of 8
--workers 2
```

This reduces:
- Partition builder memory from 200MB to 50MB
- Open file handles from 2016 to 504

**Tradeoff**: Slower processing (4× slower), but still parallel

### Solution 3: Disable index generation (TEMPORARY WORKAROUND)
**Complexity**: None | **Impact**: Eliminates user_stats completely

```bash
# Run without indexes
pbzip2 -dc ~/data/osm/changesets-251117.osm.bz2 | \
  ./target/release/changesets-to-parquet \
    --input - \
    --partition \
    --workers 8 \
    --output-dir ~/data/osm/changesets_parquet
```

Then generate indexes in a separate pass by reading the parquet files.

### Solution 4: Use external database for user_stats
**Complexity**: High | **Impact**: Best long-term solution

Use SQLite or DuckDB to accumulate user_stats:
- On-disk storage, minimal memory
- Efficient updates
- Export to parquet at the end

### Solution 5: Partition-aware worker distribution (ADVANCED)
**Complexity**: High | **Impact**: Reduces partition builder multiplication

Instead of round-robin, assign workers to partition ranges:
- Worker 0: 2005-2007
- Worker 1: 2008-2010
- etc.

This reduces per-worker partition count from 252 to ~30.

## Recommendation

**Immediate action**: Implement Solution 1 (stream user_stats to disk)

This is the best balance of:
- ✅ Fixes the primary leak
- ✅ Keeps full functionality
- ✅ Reasonable complexity
- ✅ Maintains performance

**Quick test**: Run with Solution 2 or 3 to verify the diagnosis.

## About pbzip2 and stdin

The pbzip2 piping is **not** causing the issue:
- Line 1145-1167: stdin is read as a plain stream
- pbzip2 decompresses externally, which is optimal
- The pipe buffer is negligible (~64KB-1MB kernel buffer)

## Code Changes Needed

See uncommitted changes in `src/main.rs` - you've already fixed open_changesets streaming.
The remaining work is to apply the same pattern to user_stats.

Example structure:
```rust
struct IndexCollector {
    // Current approach - stream to disk:
    user_stats_in_memory: HashMap<i64, UserStats>,  // bounded to 100K entries
    user_stats_writer: Option<ArrowWriter<File>>,
    user_stats_file_count: usize,

    // Flush when hitting threshold
    fn flush_user_stats_if_needed(&mut self) {
        if self.user_stats_in_memory.len() > 100_000 {
            self.flush_user_stats_to_disk()
        }
    }
}
```
