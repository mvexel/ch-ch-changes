"""Auth middleware — no-op when REQUIRE_AUTH=false."""

from __future__ import annotations

# When auth is fully implemented, this will:
# - Extract API key from Authorization header, X-API-Key, or query param
# - Hash and look up in SQLite
# - Check rate limits
# - Add X-RateLimit-* response headers
#
# For now, it's a pass-through.
