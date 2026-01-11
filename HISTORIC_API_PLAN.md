# OSM Changeset Historic API - Implementation Plan

## Overview

A high-performance HTTP API for querying historic OSM changeset data from Parquet files. This complements the existing replication endpoints with flexible querying capabilities.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Weekly Update Job (GitHub Actions / Local)             │
│  • Download changesets-latest.osm.bz2                   │
│  • Run changesets-to-parquet --partition                │
│  • Upload to R2                                          │
│  • Trigger VPS reload                                    │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│  Cloudflare R2 (Cold Storage)                            │
│  • Partitioned parquet files                            │
│  • Cost: ~$1.50/mo                                      │
└─────────────────────────────────────────────────────────┘
                        ↓
┌─────────────────────────────────────────────────────────┐
│  Hetzner VPS CPX21 ($8/mo)                              │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Rust Web Service (Axum + DuckDB)                 │  │
│  │  • Port 3000                                       │  │
│  │  • Systemd service                                 │  │
│  └───────────────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────────────┐  │
│  │  Local SSD: /data/changesets/                     │  │
│  │  • Parquet files synced from R2                   │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

## Project Structure

```
changesets-to-parquet/
├── src/
│   ├── main.rs              (Existing: converter tool)
│   └── bin/
│       └── api-server.rs    (NEW: API server)
├── api/                     (NEW: API server code)
│   ├── main.rs             (Entry point, Axum setup)
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── changeset.rs    (Single changeset endpoints)
│   │   ├── changesets.rs   (Batch/range endpoints)
│   │   └── meta.rs         (Metadata endpoints)
│   ├── query/
│   │   ├── mod.rs
│   │   ├── engine.rs       (DuckDB query execution)
│   │   ├── builder.rs      (SQL query builder)
│   │   └── partition.rs    (Partition resolution)
│   ├── format/
│   │   ├── mod.rs
│   │   ├── xml.rs          (OSM XML formatter)
│   │   └── json.rs         (JSON formatter)
│   ├── models/
│   │   ├── mod.rs
│   │   ├── changeset.rs    (Changeset struct)
│   │   └── metadata.rs     (Metadata struct)
│   └── lib.rs
├── Cargo.toml              (Update with API deps)
├── deploy/                 (NEW: Deployment scripts)
│   ├── systemd/
│   │   └── changeset-api.service
│   ├── nginx/
│   │   └── changeset-api.conf
│   └── update-data.sh      (R2 sync script)
└── README_API.md           (NEW: API documentation)
```

## API Endpoints

### Core Endpoints

#### Single Changeset
```
GET /changeset/:id
GET /changeset/:id.xml
GET /changeset/:id.json

Returns: Single changeset in OSM XML or JSON format
Example: /changeset/168575829
```

#### Time-based Queries
```
GET /changesets/day/:YYYYMMDD
GET /changesets/day/:YYYYMMDD/hour/:HH
GET /changesets/day/:YYYYMMDD/hour/:HH/minute/:MM

Returns: All changesets in the specified time range
Examples:
  /changesets/day/20241015
  /changesets/day/20241015/hour/14
  /changesets/day/20241015/hour/14/minute/30
```

#### ID Range Queries
```
GET /changesets/from/:id/to/:id

Returns: All changesets in the ID range (inclusive)
Example: /changesets/from/100000/to/100100
```

#### Date Range Queries
```
GET /changesets/from/:YYYYMMDD/to/:YYYYMMDD

Returns: All changesets between two dates (inclusive)
Example: /changesets/from/20241001/to/20241015
```

### Metadata Endpoints

#### Tip (Current Data Boundary)
```
GET /meta/tip

Returns:
{
  "last_updated": "2024-10-17T10:00:00Z",
  "latest_changeset_id": 168575829,
  "earliest_changeset_id": 1,
  "total_changesets": 168575829,
  "data_through": "2024-10-17T09:59:59Z",
  "update_frequency": "weekly"
}
```

#### Coverage Information
```
GET /meta/coverage

Returns:
{
  "temporal": {
    "start": "2005-04-09T19:54:13Z",
    "end": "2024-10-17T09:59:59Z"
  },
  "partitions": [
    {"year": 2024, "month": 10, "count": 1500000, "file_size_mb": 850},
    ...
  ],
  "total_changesets": 168575829
}
```

#### Health Check
```
GET /health

Returns:
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_seconds": 86400,
  "data_loaded": true,
  "partition_count": 250
}
```

### Query Parameters (All Endpoints)

```
?format=xml|json     - Response format (default: xml)
?user=username       - Filter by username
?uid=12345          - Filter by user ID
?open=true|false    - Filter open/closed changesets
?limit=1000         - Limit results (default: 10000, max: 100000)
?offset=0           - Offset for pagination
```

## Data Models

### Changeset Structure (matches Parquet schema)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changeset {
    pub id: i64,
    pub created_at: Option<i64>,      // timestamp millis
    pub closed_at: Option<i64>,       // timestamp millis
    pub open: bool,
    pub user: Option<String>,
    pub uid: Option<i64>,
    pub min_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lat: Option<f64>,
    pub max_lon: Option<f64>,
    pub num_changes: u32,
    pub comments_count: u32,
    pub description: Option<String>,
}
```

### OSM XML Output Format (Replication Compatible)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6" generator="changeset-historic-api/0.1.0">
  <changeset id="168575829"
             created_at="2024-10-15T10:30:00Z"
             closed_at="2024-10-15T10:45:00Z"
             open="false"
             user="mvexel"
             uid="12345"
             min_lat="40.7128"
             min_lon="-74.0060"
             max_lat="40.7200"
             max_lon="-73.9900"
             num_changes="42"
             comments_count="2">
    <tag k="comment" v="Fixed street names in Manhattan"/>
  </changeset>
</osm>
```

### JSON Output Format (Single Changeset)

```json
{
  "version": "0.6",
  "generator": "changeset-historic-api/0.1.0",
  "copyright": "OpenStreetMap and contributors",
  "attribution": "http://www.openstreetmap.org/copyright",
  "license": "http://opendatacommons.org/licenses/odbl/1-0/",
  "changeset": {
    "id": 168575829,
    "created_at": "2024-10-15T10:30:00Z",
    "closed_at": "2024-10-15T10:45:00Z",
    "open": false,
    "user": "mvexel",
    "uid": 12345,
    "min_lat": 40.7128,
    "min_lon": -74.0060,
    "max_lat": 40.7200,
    "max_lon": -73.9900,
    "changes_count": 42,
    "comments_count": 2,
    "tags": {
      "comment": "Fixed street names in Manhattan"
    }
  }
}
```

### JSON Output Format (List)

```json
{
  "version": "0.6",
  "generator": "changeset-historic-api/0.1.0",
  "copyright": "OpenStreetMap and contributors",
  "attribution": "http://www.openstreetmap.org/copyright",
  "license": "http://opendatacommons.org/licenses/odbl/1-0/",
  "changesets": [
    {
      "id": 168575829,
      ...
    }
  ]
}
```

## Query Optimization Strategy

### 1. Partition Pruning

For date-based queries:
```sql
-- Query: /changesets/day/20241015
-- Only scans: year=2024/month=10.parquet
SELECT * FROM read_parquet('/data/changesets/year=2024/month=10.parquet')
WHERE created_at >= '2024-10-15 00:00:00'
  AND created_at < '2024-10-16 00:00:00'
```

### 2. Index Files for Fast Lookups

Generate additional index files during conversion:

#### ID Range Index
```
/data/indexes/id-ranges.json
{
  "ranges": [
    {"min_id": 1, "max_id": 999999, "partition": "year=2005/month=04.parquet"},
    {"min_id": 1000000, "max_id": 1999999, "partition": "year=2008/month=12.parquet"},
    ...
  ]
}
```

For ID lookups, read this index first to determine which partition(s) to query.

#### User Index (Optional, for frequent user queries)
```
/data/indexes/user-stats.parquet
Columns: uid, username, changeset_count, first_changeset_id, last_changeset_id
```

### 3. Query Caching

Use an in-memory LRU cache for:
- Recent changeset IDs (last 10,000 requests)
- Metadata (tip, coverage)
- Popular user queries

## Dependencies

```toml
[dependencies]
# Existing
quick-xml = "0.36"
parquet = "53"
arrow = { version = "53", features = ["prettyprint"] }
bzip2 = "0.4"
anyhow = "1.0"
clap = { version = "4.5", features = ["derive"] }
chrono = "0.4"
serde_json = "1.0"

# NEW for API
axum = "0.7"
tokio = { version = "1", features = ["full"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }
serde = { version = "1.0", features = ["derive"] }
duckdb = { version = "1.1", features = ["bundled"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
moka = { version = "0.12", features = ["future"] }  # For caching

[dev-dependencies]
reqwest = { version = "0.12", features = ["json"] }
```

## Implementation Phases

### Phase 1: Core API (Week 1-2)
- [x] Project structure setup
- [x] DuckDB query engine wrapper
- [x] Single changeset endpoint (/changeset/:id)
- [x] Date-based endpoint (/changesets/day/:date)
- [x] XML formatter (OSM replication compatible)
- [x] JSON formatter (aligned with official OSM API)
- [x] Metadata endpoints (/meta/tip, /meta/coverage)
- [x] Health check endpoint
- [x] Basic error handling

### Phase 2: Query Features (Week 2-3)
- [x] ID range queries (/changesets/from/:id/to/:id)
- [x] Date range queries
- [x] Hour/minute precision endpoints
- [x] Query parameter filtering (user, uid, open)
- [x] Pagination (limit, offset)
- [ ] ID range index generation
- [x] Partition resolution logic

### Phase 3: Performance & Deployment (Week 3-4)
- [ ] LRU caching for hot queries
- [ ] Request logging and metrics
- [ ] Rate limiting (optional)
- [ ] Docker containerization
- [ ] Systemd service file
- [ ] Nginx reverse proxy config
- [ ] R2 sync script
- [ ] Deployment documentation

### Phase 4: Testing & Polish (Week 4+)
- [ ] Integration tests
- [ ] Load testing
- [ ] API documentation (OpenAPI/Swagger)
- [ ] Example queries documentation
- [ ] Monitoring setup (uptime, response times)

## Deployment Strategy

### VPS Setup (Hetzner CPX21)

1. **Initial Setup**
   ```bash
   # Install dependencies
   sudo apt update && sudo apt install -y nginx rclone

   # Install Rust
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

   # Create data directory
   sudo mkdir -p /data/changesets
   sudo chown -R $USER:$USER /data
   ```

2. **Download Parquet Data**
   ```bash
   # Configure rclone for R2
   rclone config

   # Sync data
   rclone sync r2:changesets-osm-lol/data/ /data/changesets/ -P
   ```

3. **Build and Deploy API**
   ```bash
   # Clone repo
   git clone https://github.com/mvexel/changesets-to-parquet.git
   cd changesets-to-parquet

   # Build API server
   cargo build --release --bin api-server

   # Install systemd service
   sudo cp deploy/systemd/changeset-api.service /etc/systemd/system/
   sudo systemctl enable changeset-api
   sudo systemctl start changeset-api
   ```

4. **Configure Nginx**
   ```bash
   sudo cp deploy/nginx/changeset-api.conf /etc/nginx/sites-available/
   sudo ln -s /etc/nginx/sites-available/changeset-api.conf /etc/nginx/sites-enabled/
   sudo nginx -t
   sudo systemctl reload nginx
   ```

### Weekly Update Job

```bash
#!/bin/bash
# deploy/update-data.sh

set -e

echo "Starting weekly data update..."

# Download latest dump
wget -O /tmp/changesets-latest.osm.bz2 \
  https://planet.osm.org/planet/changesets-latest.osm.bz2

# Convert to parquet
./target/release/changesets-to-parquet \
  --input /tmp/changesets-latest.osm.bz2 \
  --output-dir /tmp/changesets-output \
  --partition \
  --generate-indexes

# Upload to R2
rclone sync /tmp/changesets-output r2:changesets-osm-lol/data/ \
  --progress --transfers 4

# Download to VPS
ssh vps "cd /data && rclone sync r2:changesets-osm-lol/data/ ./changesets/ --progress"

# Restart API service
ssh vps "sudo systemctl restart changeset-api"

echo "Update complete!"
```

## Cost Breakdown

| Component | Cost (Monthly) |
|-----------|----------------|
| Hetzner CPX21 VPS | $8.00 |
| Cloudflare R2 (100GB) | $1.50 |
| Domain (if new) | $1.00 |
| **Total** | **$10.50** |

## Performance Expectations

### Hardware: Hetzner CPX21
- 3 vCPU, 4GB RAM, 80GB SSD
- Expected throughput: **50-100 req/sec**
- Query latency:
  - Single changeset by ID: **10-50ms** (with index)
  - Day query (1000 changesets): **50-200ms**
  - Month query (50k changesets): **500ms-2s**

### Optimization Opportunities
1. **Caching**: Cache hot queries (recent IDs, popular users)
2. **Indexing**: Pre-build ID range indexes
3. **Connection pooling**: Reuse DuckDB connections
4. **Compression**: Enable gzip for responses
5. **CDN**: Put Cloudflare in front for static responses

## Future Enhancements

### Hot API Integration
When ready for real-time updates:
- Separate endpoint: `/changesets/recent` (last hour/day)
- Tracks OSM replication minutely diffs
- Stores in PostgreSQL or in-memory DuckDB
- Smart router: ID > tip → hot API, else → historic API

### Additional Features
- [ ] Spatial queries (bbox)
- [ ] Tag-based filtering
- [ ] Aggregation endpoints (stats, counts)
- [ ] GraphQL API
- [ ] WebSocket streaming for live updates
- [ ] OpenAPI/Swagger documentation
- [ ] Multi-region deployment

## Success Metrics

### MVP Launch Goals
- ✓ API responds to basic queries
- ✓ Compatible with OSM XML format
- ✓ 99% uptime
- ✓ < 500ms p95 latency
- ✓ Costs under $15/mo

### Growth Targets (6 months)
- 1000+ queries/day
- < 200ms p95 latency
- 99.5% uptime
- Community adoption (OSM tools integrating)

## Documentation

### API Documentation
- OpenAPI/Swagger spec
- Interactive API explorer
- Example queries for common use cases
- Client libraries (Python, JavaScript)

### Deployment Documentation
- VPS setup guide
- R2 configuration
- Update automation
- Monitoring setup
- Troubleshooting guide

## Testing Strategy

### Unit Tests
- Query builder logic
- Partition resolution
- Date parsing
- XML/JSON formatters

### Integration Tests
- End-to-end API requests
- Query correctness
- Format compatibility
- Error handling

### Load Tests
- Sustained 50 req/sec
- Burst to 200 req/sec
- Concurrent user queries
- Large result sets

## Security Considerations

- [ ] Rate limiting per IP
- [ ] Input validation (prevent SQL injection)
- [ ] CORS configuration
- [ ] Admin endpoints authentication
- [ ] HTTPS only (via Nginx)
- [ ] Regular security updates

## Monitoring

### Metrics to Track
- Request rate (req/sec)
- Response times (p50, p95, p99)
- Error rates (4xx, 5xx)
- Query types distribution
- Cache hit rates
- Disk usage
- Memory usage

### Alerting
- Service down
- High error rate (> 5%)
- High latency (p95 > 1s)
- Disk space low (< 10GB free)

## Questions / Decisions Needed

1. **Domain name**: What domain will this be hosted on?
   - Suggestion: `changesets.osm.lol` or `api.changesets.osm.lol`

2. **Update frequency**: Weekly or monthly for MVP?
   - Recommendation: Weekly to start

3. **API versioning**: Should we version the API from the start?
   - Suggestion: `/v1/changeset/:id` for future flexibility

4. **Authentication**: Open API or require API keys?
   - Recommendation: Open for MVP, add keys later if needed

5. **Result limits**: What's a reasonable max limit per query?
   - Recommendation: 100k changesets max per query
