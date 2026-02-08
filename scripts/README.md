# Deployment Scripts

**See [/DEPLOYMENT.md](../DEPLOYMENT.md) for the comprehensive deployment guide.**

This directory contains scripts for systemd-based deployment. For Docker deployment (recommended), see `docker/` subdirectory.

## Quick Reference

### Systemd Deployment Scripts

- **`setup.sh`** - One-time server setup (creates user, directories, installs services)
- **`deploy.sh`** - Deploy application updates
- **`init-data.sh`** - Load initial planet dump data
- **`healthcheck.sh`** - Monitor service health (use with cron)
- **`check-integrity.sh`** - Verify parquet data integrity

### Docker Deployment Scripts

See `docker/` subdirectory:
- **`docker/init-data.sh`** - Load initial data using Docker converter

## Usage

For complete deployment instructions, workflows, and troubleshooting, see:

**→ [/DEPLOYMENT.md](../DEPLOYMENT.md)**
