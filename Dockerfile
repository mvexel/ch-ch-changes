# Multi-stage build for Changeset Historic API
FROM rust:slim as builder

WORKDIR /app

# Update to latest Rust and install build dependencies
RUN rustup update stable && \
    apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY api ./api

# Build the API server in release mode
RUN cargo build --release --bin changeset-server

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/changeset-server /usr/local/bin/changeset-server

# Create data directory
RUN mkdir -p /data/changesets

# Set environment variable for data path
ENV DATA_PATH=/data/changesets

# Expose port
EXPOSE 3000

# Run the server
CMD ["changeset-server"]
