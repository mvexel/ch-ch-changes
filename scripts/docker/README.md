# Docker Deployment Scripts

**See [/DEPLOYMENT.md](../../DEPLOYMENT.md) for the comprehensive deployment guide.**

This directory contains Docker-specific deployment scripts. Docker is the **recommended** deployment method.

## Quick Reference

- **`init-data.sh`** - Load initial planet dump data using Docker converter

## Usage

For complete Docker deployment instructions, see:

**→ [/DEPLOYMENT.md](../../DEPLOYMENT.md#docker-deployment-recommended)**

## Quick Start

```bash
# 1. Load initial data
./scripts/docker/init-data.sh

# 2. Start services
docker-compose up -d

# 3. Verify
curl http://localhost:3000/health
```
