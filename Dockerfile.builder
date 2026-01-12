FROM rust:slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    g++ \
    && rm -rf /var/lib/apt/lists/*

# Copy source and build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY api ./api

RUN cargo build --release --bin changesets-to-parquet

# Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    wget \
    bzip2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/changesets-to-parquet /usr/local/bin/
COPY scripts/process_changesets.sh /app/scripts/process_changesets.sh
RUN chmod +x /app/scripts/process_changesets.sh

RUN mkdir -p /data/downloads /data/temp_changesets

CMD ["/app/scripts/process_changesets.sh"]
