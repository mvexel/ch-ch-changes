"""Replication worker — fetches OSM changeset diffs and writes Parquet."""

from __future__ import annotations

import argparse
import logging
import os
import time
from datetime import datetime, timezone
from pathlib import Path

from ch_ch_changes.config import WorkerConfig
from ch_ch_changes.core.metadata import get_metadata_path, update_metadata_incremental
from ch_ch_changes.core.parsing import parse_changesets
from ch_ch_changes.core.writer import write_partitioned
from ch_ch_changes.models import Changeset
from ch_ch_changes.worker.compaction import compact_old_deltas
from ch_ch_changes.worker.fetcher import fetch_sequence, fetch_state

logger = logging.getLogger(__name__)


class HourlyBatch:
    """Accumulates changesets for an hour, flushing on hour boundary."""

    def __init__(self):
        self.changesets: list[Changeset] = []
        self.batch_hour: str | None = None
        self.start_sequence: int | None = None
        self.end_sequence: int | None = None
        self.sequence_count: int = 0

    def add(self, changesets: list[Changeset], sequence: int, timestamp: str) -> bool:
        """Add changesets. Returns True if hour boundary was crossed."""
        hour = timestamp[:13].replace("-", "").replace("T", "").replace(":", "")[:10]

        if self.batch_hour is None:
            self.batch_hour = hour
            self.start_sequence = sequence

        boundary_crossed = hour != self.batch_hour

        if boundary_crossed:
            return True

        self.changesets.extend(changesets)
        self.end_sequence = sequence
        self.sequence_count += 1
        return False

    def take_batch(self) -> tuple[list[Changeset], str]:
        """Take the current batch, resetting for a new hour."""
        cs = self.changesets
        hour = self.batch_hour or ""
        self.changesets = []
        self.batch_hour = None
        self.start_sequence = None
        self.end_sequence = None
        self.sequence_count = 0
        return cs, hour

    def flush(self) -> tuple[list[Changeset], str]:
        """Get remaining changesets without full reset."""
        return self.take_batch()

    def __len__(self) -> int:
        return len(self.changesets)


def _write_state(state_file: Path, sequence: int, last_run: str) -> None:
    """Write state.yaml atomically."""
    state_file.parent.mkdir(parents=True, exist_ok=True)
    tmp = state_file.with_suffix(".yaml.tmp")
    tmp.write_text(f"---\nsequence: {sequence}\nlast_run: {last_run}\n")
    os.replace(tmp, state_file)


def _load_local_state(state_file: Path) -> dict | None:
    """Load local state.yaml if it exists."""
    if not state_file.exists():
        return None
    content = state_file.read_text()
    result = {}
    for line in content.splitlines():
        line = line.strip()
        if line.startswith("sequence:"):
            result["sequence"] = int(line.split(":", 1)[1].strip())
        elif line.startswith("last_run:"):
            result["last_run"] = line.split(":", 1)[1].strip()
    return result if "sequence" in result else None


def _bootstrap_from_parquet(data_path: Path) -> int | None:
    """Try to find the latest replication_sequence from existing Parquet data."""
    import duckdb

    globs = []
    for subdir in ("delta", "base"):
        d = data_path / subdir
        if d.exists() and any(d.rglob("*.parquet")):
            globs.append(str(d / "**/*.parquet"))

    if not globs:
        return None

    try:
        conn = duckdb.connect(":memory:")
        source = ", ".join(f"'{g}'" for g in globs)
        sql = (
            f"SELECT MAX(replication_sequence) "
            f"FROM read_parquet([{source}], hive_partitioning=true)"
        )
        row = conn.execute(sql).fetchone()
        conn.close()
        if row and row[0] is not None:
            return int(row[0])
    except Exception as e:
        logger.warning("Bootstrap from parquet failed: %s", e)
    return None


def run_replication(args: argparse.Namespace, config: WorkerConfig) -> None:
    """Main replication loop."""
    delta_dir = config.get_delta_dir()
    state_file = config.get_state_file()
    tmp_dir = config.get_tmp_dir()
    base_url = config.CHANGESET_REPLICATION_URL
    metadata_path = get_metadata_path(Path(config.DATA_PATH))

    delta_dir.mkdir(parents=True, exist_ok=True)
    tmp_dir.mkdir(parents=True, exist_ok=True)

    # Determine starting sequence
    local_state = _load_local_state(state_file)
    if local_state:
        local_sequence = local_state["sequence"]
        logger.info("Resuming from local state: sequence %d", local_sequence)
    elif args.start_sequence is not None:
        local_sequence = args.start_sequence
        logger.info("Starting from explicit sequence: %d", local_sequence)
    elif not args.no_bootstrap:
        bootstrapped = _bootstrap_from_parquet(Path(config.DATA_PATH))
        if bootstrapped is not None:
            local_sequence = bootstrapped
            logger.info("Bootstrapped from parquet: sequence %d", local_sequence)
        else:
            remote = fetch_state(base_url)
            local_sequence = remote["sequence"]
            logger.info("Starting from remote latest: sequence %d", local_sequence)
    else:
        remote = fetch_state(base_url)
        local_sequence = remote["sequence"]
        logger.info("Starting from remote latest: sequence %d", local_sequence)

    while True:
        # Fetch remote state
        remote = fetch_state(base_url)
        remote_sequence = remote["sequence"]
        # OSM state files are off-by-one
        end_sequence = remote_sequence + 1

        if local_sequence >= end_sequence:
            if args.once:
                logger.info("Caught up at sequence %d, exiting (--once)", local_sequence)
                return
            logger.info(
                "Caught up at sequence %d, sleeping %ds",
                local_sequence,
                config.POLL_INTERVAL_SECONDS,
            )
            time.sleep(config.POLL_INTERVAL_SECONDS)
            continue

        sequences_to_process = end_sequence - local_sequence
        if config.MAX_SEQUENCES_PER_RUN:
            sequences_to_process = min(sequences_to_process, config.MAX_SEQUENCES_PER_RUN)
            end_sequence = local_sequence + sequences_to_process

        logger.info(
            "Processing sequences %d -> %d (%d sequences)",
            local_sequence,
            end_sequence - 1,
            sequences_to_process,
        )

        batch = HourlyBatch() if config.HOURLY_BATCHING else None
        max_batch_sequences = args.max_batch_sequences

        seq = local_sequence
        while seq < end_sequence:
            seq += 1
            try:
                # Fetch and parse
                xml_path = fetch_sequence(base_url, seq, tmp_dir)
                changesets = list(
                    parse_changesets(str(xml_path), continue_on_error=args.continue_on_error)
                )
                xml_path.unlink(missing_ok=True)

                if not changesets:
                    logger.debug("Sequence %d: no changesets", seq)
                    continue

                # Set replication_sequence on all changesets
                for cs in changesets:
                    cs.replication_sequence = seq

                # Get timestamp for this sequence
                seq_state = fetch_state(base_url, seq)
                seq_timestamp = seq_state.get("last_run", datetime.now(timezone.utc).isoformat())

                if batch is not None:
                    # Hourly batching mode
                    boundary = batch.add(changesets, seq, seq_timestamp)
                    if boundary or batch.sequence_count >= max_batch_sequences:
                        # Flush current batch
                        batch_cs, hour = batch.take_batch()
                        if batch_cs:
                            filename = f"data_{hour}.parquet"
                            stats = write_partitioned(batch_cs, delta_dir, filename=filename)
                            update_metadata_incremental(metadata_path, stats)
                            logger.info("Wrote %d changesets for hour %s", len(batch_cs), hour)
                        # Re-add the current changesets (they're for the new hour)
                        batch.add(changesets, seq, seq_timestamp)
                else:
                    # Per-sequence mode
                    stats = write_partitioned(changesets, delta_dir)
                    update_metadata_incremental(metadata_path, stats)

                # Update state
                last_run = seq_timestamp
                _write_state(state_file, seq, last_run)

            except Exception:
                logger.exception("Error processing sequence %d", seq)
                if not args.continue_on_error:
                    raise
                continue

        # Flush remaining batch
        if batch is not None and len(batch) > 0:
            batch_cs, hour = batch.flush()
            if batch_cs:
                filename = f"data_{hour}.parquet"
                stats = write_partitioned(batch_cs, delta_dir, filename=filename)
                update_metadata_incremental(metadata_path, stats)
                logger.info("Flushed %d remaining changesets for hour %s", len(batch_cs), hour)

        local_sequence = seq

        # Optional compaction
        if config.ENABLE_COMPACTION:
            compacted = compact_old_deltas(delta_dir, config.COMPACTION_AGE_HOURS)
            if compacted:
                logger.info("Compacted %d old delta files", compacted)

        if args.once:
            logger.info("Finished single pass at sequence %d", local_sequence)
            return

        # Clean temp dir
        if not args.keep_temp:
            for f in tmp_dir.glob("*"):
                f.unlink(missing_ok=True)


def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    config = WorkerConfig()

    parser = argparse.ArgumentParser(description="ch-ch-changes replication worker")
    parser.add_argument("--start-sequence", type=int, default=config.REPLICATION_START_SEQUENCE)
    parser.add_argument("--once", action="store_true", help="Exit after single catch-up pass")
    parser.add_argument("--no-bootstrap", action="store_true", help="Disable parquet bootstrap")
    parser.add_argument("--batch-size", type=int, default=10000)
    parser.add_argument("--continue-on-error", action="store_true")
    parser.add_argument("--keep-temp", action="store_true")
    parser.add_argument("--max-batch-sequences", type=int, default=120)

    args = parser.parse_args()
    run_replication(args, config)


if __name__ == "__main__":
    main()
