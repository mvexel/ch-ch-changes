"""Download replication sequences from OSM."""

from __future__ import annotations

import gzip
import logging
import time
from pathlib import Path
from typing import Optional

import httpx

logger = logging.getLogger(__name__)

REQUEST_TIMEOUT = 180.0
CONNECT_TIMEOUT = 30.0
MAX_RETRIES = 5
INITIAL_BACKOFF = 2.0


def sequence_to_path(sequence: int) -> str:
    """Convert sequence number to OSM's AAA/BBB/CCC path format."""
    s = f"{sequence:09d}"
    return f"{s[:3]}/{s[3:6]}/{s[6:]}"


def fetch_state(base_url: str, sequence: Optional[int] = None) -> dict:
    """Download and parse a state.yaml file.

    Args:
        base_url: Replication base URL.
        sequence: If given, fetch state for this specific sequence.
                  Otherwise fetch the latest state.

    Returns:
        Dict with 'sequence' and 'last_run' keys.
    """
    if sequence is not None:
        url = f"{base_url.rstrip('/')}/{sequence_to_path(sequence)}.state.txt"
    else:
        url = f"{base_url.rstrip('/')}/state.yaml"

    text = _fetch_text(url)

    result = {}
    for line in text.splitlines():
        line = line.strip()
        if not line or line == "---":
            continue
        if line.startswith("sequence:") or line.startswith("sequenceNumber:"):
            key, val = line.split(":", 1) if ":" in line else (line, "")
            result["sequence"] = int(val.strip())
        elif "=" in line and line.startswith("sequenceNumber"):
            result["sequence"] = int(line.split("=", 1)[1].strip())
        elif line.startswith("last_run:") or line.startswith("timestamp"):
            if ":" in line:
                result["last_run"] = line.split(":", 1)[1].strip()
            elif "=" in line:
                result["last_run"] = line.split("=", 1)[1].strip().strip("\\")

    return result


def fetch_sequence(base_url: str, sequence: int, tmp_dir: Path) -> Path:
    """Download a replication sequence .osm.gz file, return path to decompressed XML.

    Args:
        base_url: Replication base URL.
        sequence: Sequence number to download.
        tmp_dir: Directory for temporary files.

    Returns:
        Path to the downloaded (decompressed) XML file.
    """
    url = f"{base_url.rstrip('/')}/{sequence_to_path(sequence)}.osm.gz"
    tmp_dir.mkdir(parents=True, exist_ok=True)
    gz_path = tmp_dir / f"{sequence}.osm.gz"
    xml_path = tmp_dir / f"{sequence}.osm"

    data = _fetch_bytes(url)
    gz_path.write_bytes(data)

    # Decompress
    with gzip.open(gz_path, "rb") as f_in:
        xml_path.write_bytes(f_in.read())

    gz_path.unlink()
    return xml_path


def _fetch_text(url: str) -> str:
    return _fetch_with_retry(url).text


def _fetch_bytes(url: str) -> bytes:
    return _fetch_with_retry(url).content


def _fetch_with_retry(url: str) -> httpx.Response:
    """Fetch URL with exponential backoff retry on transient errors."""
    backoff = INITIAL_BACKOFF
    last_exc = None

    for attempt in range(MAX_RETRIES + 1):
        try:
            with httpx.Client(
                timeout=httpx.Timeout(REQUEST_TIMEOUT, connect=CONNECT_TIMEOUT),
                headers={"User-Agent": "changeset-replication-worker/0.1.0"},
                follow_redirects=True,
            ) as client:
                resp = client.get(url)
                if resp.status_code == 429 or resp.status_code >= 500:
                    raise httpx.HTTPStatusError(
                        f"Server error {resp.status_code}",
                        request=resp.request,
                        response=resp,
                    )
                resp.raise_for_status()
                return resp
        except (httpx.TimeoutException, httpx.ConnectError, httpx.HTTPStatusError) as e:
            last_exc = e
            if attempt < MAX_RETRIES:
                logger.warning(
                    "Fetch %s failed (attempt %d/%d): %s. Retrying in %.0fs",
                    url,
                    attempt + 1,
                    MAX_RETRIES,
                    e,
                    backoff,
                )
                time.sleep(backoff)
                backoff *= 2
            else:
                raise

    raise last_exc  # type: ignore[misc]
