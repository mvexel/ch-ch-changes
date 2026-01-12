# Builder instance for processing OSM changeset archives
FROM rust:slim-bookworm as builder

WORKDIR /app

# Update to latest Rust and install build dependencies
RUN rustup update stable && \
    apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY api ./api

# Build only the CLI tool in release mode
RUN cargo build --release --bin changesets-to-parquet

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies including download tools
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libstdc++6 \
    wget \
    curl \
    bzip2 \
    && rm -rf /var/lib/apt/lists/*

# Copy the CLI binary from builder
COPY --from=builder /app/target/release/changesets-to-parquet /usr/local/bin/changesets-to-parquet

# Copy processing script
COPY scripts/process_changesets.sh /app/scripts/process_changesets.sh
RUN chmod +x /app/scripts/process_changesets.sh

# Create data directories
RUN mkdir -p /data/downloads /data/temp_changesets

# Default command runs the processing script
CMD ["/app/scripts/process_changesets.sh"]
