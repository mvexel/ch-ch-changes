# Changeset Replication Worker

This worker pulls the OSM **minutely changeset replication** feed and appends new Parquet files
to the delta dataset. It is intended to keep data fresh between full refreshes.

## Output Layout

The worker writes delta files under:

```
$DATA_PATH/delta/
  year=YYYY/
    month=MM/
      seq-XXXXXXXXX.parquet
  state.yaml
  tmp/
```

Each `seq-XXXXXXXXX.parquet` file contains the changesets from one replication sequence.

## First-Run Bootstrapping

On the first run, there is no local `state.yaml`. Set **one** of:

- `REPLICATION_START_SEQUENCE` (recommended)
- `REPLICATION_START_TIMESTAMP` (RFC3339, e.g. `2025-01-15T00:00:00Z`)

After the first successful run, the worker writes `state.yaml` and continues from there.

## Example (systemd)

1) Install the binary:

```
cargo build --release --bin changeset-replication-worker
cp target/release/changeset-replication-worker /opt/changeset-server/bin/
```

2) Install the unit:

```
cp systemd/changeset-replication-worker.service /etc/systemd/system/
systemctl daemon-reload
```

3) Configure environment (edit the unit or drop-in):

```
Environment="DATA_PATH=/opt/changeset-data/changesets"
Environment="POLL_INTERVAL_SECONDS=60"
Environment="REPLICATION_START_SEQUENCE=123456789"
```

4) Start and enable:

```
systemctl enable --now changeset-replication-worker
```

## Manual Run

```
DATA_PATH=/opt/changeset-data/changesets \
REPLICATION_START_SEQUENCE=123456789 \
./changeset-replication-worker --once
```

## Bootstrap State From Parquet

If you have a fresh base snapshot and want to initialize the replication state
from it, run:

```
DATA_PATH=/opt/changeset-data/changesets \
./changeset-replication-worker --bootstrap-state-from-parquet
```

## Notes

- The OSM changeset replication feed is **minutely only**.
- The root state file is `state.yaml`; per-sequence state files are `.state.txt` but use the same YAML format.
- If you reset your base snapshot, delete `$DATA_PATH/delta` and re-seed `REPLICATION_START_SEQUENCE`.
