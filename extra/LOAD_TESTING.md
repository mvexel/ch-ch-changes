# Load Testing Guide

This guide focuses on repeatable, scenario-based API load testing.

## Recommended Approach (k6)

Use k6 to mix endpoints and ramp traffic while tracking p95/p99 latency,
error rate, and throughput. The scenario uses environment variables so you
can target real data.

1) Adjust date/time/id to values that exist in your dataset.
2) Run a warm-up first to avoid cold cache skew.
3) Capture server metrics during the run (CPU, RSS, disk IO, open files).

### Script
- File: `extra/k6-loadtest.js`

### Run
```bash
k6 run \
  -e BASE_URL=http://localhost:3000 \
  -e DATE=20240101 \
  -e HOUR=12 \
  -e MIN=30 \
  -e ID=12345 \
  extra/k6-loadtest.js
```

### Example Thresholds
- `http_req_failed < 1%`
- `p95 < 500ms`
- `p99 < 1200ms`

Tune these to your hardware and acceptable SLOs.

## Quick Single-Endpoint Throughput (optional)
If you only need a quick throughput number for a single endpoint:

```bash
# Example: /health
wrk -t4 -c200 -d30s http://localhost:3000/health

# Example: /changeset/{id}.json
wrk -t4 -c100 -d30s http://localhost:3000/changeset/12345.json
```

## Notes
- Use a separate load generator when possible to avoid starving the API host.
- Keep heavy endpoints (like `/changesets/from/.../to/...`) at low weight.
- Consider a steady-state test after the ramp to see sustained behavior.
