#!/bin/bash
set -euo pipefail

# Configuration
HEALTH_URL="http://localhost:3000/health"
SERVICE_NAME="changeset-server"
ALERT_EMAIL="${ALERT_EMAIL:-}"  # Set via environment or cron

# Exit codes
EXIT_OK=0
EXIT_SERVICE_DOWN=1
EXIT_HEALTH_FAILED=2

# Check if service is running
if ! systemctl is-active --quiet "$SERVICE_NAME"; then
    MESSAGE="ALERT: $SERVICE_NAME is not running"
    echo "$MESSAGE"
    logger -t healthcheck "$MESSAGE"

    if [ -n "$ALERT_EMAIL" ]; then
        echo "$MESSAGE" | mail -s "Alert: $SERVICE_NAME down" "$ALERT_EMAIL"
    fi

    exit $EXIT_SERVICE_DOWN
fi

# Check health endpoint
if ! curl -sf --max-time 5 "$HEALTH_URL" > /dev/null 2>&1; then
    MESSAGE="ALERT: $SERVICE_NAME health check failed at $HEALTH_URL"
    echo "$MESSAGE"
    logger -t healthcheck "$MESSAGE"

    # Get recent logs for context
    LOGS=$(journalctl -u "$SERVICE_NAME" -n 10 --no-pager)

    if [ -n "$ALERT_EMAIL" ]; then
        {
            echo "$MESSAGE"
            echo ""
            echo "Recent logs:"
            echo "$LOGS"
        } | mail -s "Alert: $SERVICE_NAME unhealthy" "$ALERT_EMAIL"
    fi

    exit $EXIT_HEALTH_FAILED
fi

# All checks passed
echo "OK: $SERVICE_NAME is healthy"
exit $EXIT_OK
