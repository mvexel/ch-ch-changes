FROM python:3.11-slim

# Install system dependencies for lxml, DuckDB, and healthcheck
RUN apt-get update && apt-get install -y \
    libxml2 \
    libxslt1.1 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy project files
COPY pyproject.toml ./
COPY src/ ./src/

# Install Python dependencies
RUN pip install --no-cache-dir -e .

# Create data directory
RUN mkdir -p /data

# Default environment variables
ENV DATA_PATH=/data
ENV HOST=0.0.0.0
ENV PORT=3000
ENV LOG_LEVEL=INFO

# Expose server port
EXPOSE 3000

# Entry point will be overridden by docker-compose or command
ENTRYPOINT ["python", "-m"]
