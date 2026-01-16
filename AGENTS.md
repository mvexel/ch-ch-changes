# Agent Notes

This repo contains two Rust binaries:
- `changesets-to-parquet` (converter) in `src/main.rs`
- `changeset-server` (API) in `api/main.rs`

## Common Commands
- Build all: `cargo build --release`
- Run converter: `changesets-to-parquet --input changesets-latest.osm.bz2 --output changesets.parquet`
- Run API server: `DATA_PATH=/opt/changeset-data/changesets ./changeset-server`

## API Endpoints (from README)
- `GET /health`
- `GET /changeset/{id}.json`
- `GET /changesets/day/{YYYYMMDD}`
- `GET /changesets/day/{YYYYMMDD}/hour/{HH}/minute/{MM}`
- `GET /changesets/from/{start}/to/{end}`
- `GET /meta/tip`
- `GET /meta/coverage`

## Docs
- Server deployment and ops notes: `docs/SERVING.md`
- Hetzner one-shot processing: `docs/HETZNER.md`

## Notes
- API server is read-only and serves Parquet data from `DATA_PATH`.
- Date-range endpoints can be large; use with care when load testing.
