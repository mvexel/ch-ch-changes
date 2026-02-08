"""Tests for worker components."""

from ch_ch_changes.models import Changeset
from ch_ch_changes.worker.fetcher import sequence_to_path
from ch_ch_changes.worker.main import HourlyBatch


def test_sequence_to_path():
    assert sequence_to_path(1) == "000/000/001"
    assert sequence_to_path(123) == "000/000/123"
    assert sequence_to_path(1234) == "000/001/234"
    assert sequence_to_path(6347891) == "006/347/891"


def test_hourly_batch_same_hour():
    batch = HourlyBatch()
    cs = [Changeset(id=1, open=True)]
    assert batch.add(cs, 100, "2024-05-01T10:30:00Z") is False
    assert len(batch) == 1

    cs2 = [Changeset(id=2, open=True)]
    assert batch.add(cs2, 101, "2024-05-01T10:45:00Z") is False
    assert len(batch) == 2


def test_hourly_batch_boundary():
    batch = HourlyBatch()
    cs1 = [Changeset(id=1, open=True)]
    batch.add(cs1, 100, "2024-05-01T10:30:00Z")

    cs2 = [Changeset(id=2, open=True)]
    crossed = batch.add(cs2, 101, "2024-05-01T11:05:00Z")
    assert crossed is True

    # Take the old batch
    old_cs, hour = batch.take_batch()
    assert len(old_cs) == 1
    assert old_cs[0].id == 1
    assert hour == "2024050110"


def test_hourly_batch_flush():
    batch = HourlyBatch()
    cs = [Changeset(id=1, open=True)]
    batch.add(cs, 100, "2024-05-01T10:30:00Z")

    flushed, hour = batch.flush()
    assert len(flushed) == 1
    assert len(batch) == 0
