"""Download replication sequences from OSM."""

from __future__ import annotations

import asyncio
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
PARALLEL_FETCHES = 10


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


async def _fetch_with_retry_async(url: str, client: httpx.AsyncClient) -> httpx.Response:
    """Fetch URL with exponential backoff retry on transient errors (async)."""
    backoff = INITIAL_BACKOFF
    last_exc = None

    for attempt in range(MAX_RETRIES + 1):
        try:
            resp = await client.get(url)
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
                await asyncio.sleep(backoff)
                backoff *= 2
            else:
                raise

    raise last_exc  # type: ignore[misc]


async def fetch_sequence_async(
    base_url: str, sequence: int, tmp_dir: Path, client: httpx.AsyncClient
) -> tuple[int, Path, dict]:
    """Download a sequence asynchronously.

    Returns:
        Tuple of (sequence, xml_path, state_dict)
    """
    url = f"{base_url.rstrip('/')}/{sequence_to_path(sequence)}.osm.gz"
    state_url = f"{base_url.rstrip('/')}/{sequence_to_path(sequence)}.state.txt"

    # Fetch both .osm.gz and .state.txt in parallel
    osm_resp, state_resp = await asyncio.gather(
        _fetch_with_retry_async(url, client), _fetch_with_retry_async(state_url, client)
    )

    # Decompress OSM data (gzip.decompress works on bytes directly)
    xml_path = tmp_dir / f"{sequence}.osm"
    xml_data = gzip.decompress(osm_resp.content)
    xml_path.write_bytes(xml_data)

    # Parse state
    state = {}
    for line in state_resp.text.splitlines():
        line = line.strip()
        if not line:
            continue
        if "=" in line and line.startswith("timestamp"):
            state["last_run"] = line.split("=", 1)[1].strip().strip("\\")
        elif ":" in line and line.startswith("timestamp"):
            state["last_run"] = line.split(":", 1)[1].strip()

    return sequence, xml_path, state


async def fetch_sequences_parallel(
    base_url: str, sequences: list[int], tmp_dir: Path, concurrency: int = PARALLEL_FETCHES
) -> list[tuple[int, Path, dict]]:
    """Fetch multiple sequences in parallel.

    Args:
        base_url: Replication base URL.
        sequences: List of sequence numbers to fetch.
        tmp_dir: Directory for temporary files.
        concurrency: Number of parallel fetches.

    Returns:
        List of (sequence, xml_path, state_dict) tuples, in order.
    """
    tmp_dir.mkdir(parents=True, exist_ok=True)

    async with httpx.AsyncClient(
        timeout=httpx.Timeout(REQUEST_TIMEOUT, connect=CONNECT_TIMEOUT),
        headers={"User-Agent": "changeset-replication-worker/0.1.0"},
        follow_redirects=True,
        limits=httpx.Limits(max_connections=concurrency, max_keepalive_connections=concurrency),
    ) as client:
        # Use a semaphore to limit concurrency
        sem = asyncio.Semaphore(concurrency)

        async def fetch_with_sem(seq):
            async with sem:
                return await fetch_sequence_async(base_url, seq, tmp_dir, client)

        results = await asyncio.gather(*[fetch_with_sem(seq) for seq in sequences])

    return results
