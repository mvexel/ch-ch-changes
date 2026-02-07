"""Integration tests for FastAPI routes."""

import json


def test_health(client):
    resp = client.get("/health")
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "ok"
    assert data["data_loaded"] is True
    assert data["partition_count"] >= 1


def test_changesets_by_day_json(client):
    resp = client.get("/changesets/day/20240501?format=json")
    assert resp.status_code == 200
    assert resp.headers["content-type"] == "application/json"
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 1
    assert data["changesets"][0]["id"] == 1


def test_changesets_by_day_xml(client):
    resp = client.get("/changesets/day/20240501")
    assert resp.status_code == 200
    assert resp.headers["content-type"] == "application/xml"
    assert "<changeset" in resp.text
    assert 'id="1"' in resp.text


def test_changesets_by_day_jsonl(client):
    resp = client.get("/changesets/day/20240502.jsonl")
    assert resp.status_code == 200
    assert resp.headers["content-type"] == "application/x-ndjson"
    lines = [line for line in resp.text.strip().split("\n") if line]
    assert len(lines) == 1
    assert json.loads(lines[0])["id"] == 2


def test_changesets_by_day_invalid_date(client):
    resp = client.get("/changesets/day/notadate")
    assert resp.status_code == 400


def test_changesets_by_hour(client):
    resp = client.get("/changesets/day/20240502/hour/12?format=json")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 1
    assert data["changesets"][0]["id"] == 2


def test_changesets_by_hour_invalid(client):
    resp = client.get("/changesets/day/20240502/hour/25")
    assert resp.status_code == 400


def test_changesets_by_minute(client):
    resp = client.get("/changesets/day/20240501/hour/00/minute/00?format=json")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 1


def test_changesets_by_minute_invalid(client):
    resp = client.get("/changesets/day/20240501/hour/00/minute/60")
    assert resp.status_code == 400


def test_changesets_by_date_range(client):
    resp = client.get("/changesets/date/from/20240501/to/20240502?format=json")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 2


def test_changesets_date_range_too_large(client):
    resp = client.get("/changesets/date/from/20240501/to/20240510")
    assert resp.status_code == 400


def test_changesets_by_id_range(client):
    resp = client.get("/changesets/id/from/1/to/2?format=json")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 2


def test_changesets_id_range_invalid_start(client):
    resp = client.get("/changesets/id/from/abc/to/100")
    assert resp.status_code == 400


def test_changesets_id_range_inverted(client):
    resp = client.get("/changesets/id/from/100/to/50")
    assert resp.status_code == 400


def test_changesets_id_range_too_large(client):
    resp = client.get("/changesets/id/from/1/to/2000000")
    assert resp.status_code == 400


def test_changesets_id_range_with_format_extension(client):
    resp = client.get("/changesets/id/from/1/to/2.json")
    assert resp.status_code == 200
    assert resp.headers["content-type"] == "application/json"


def test_changesets_filter_by_user(client):
    resp = client.get("/changesets/id/from/1/to/2?format=json&user=alice")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert len(data["changesets"]) == 1
    assert data["changesets"][0]["user"] == "alice"


def test_changesets_filter_by_open(client):
    resp = client.get("/changesets/id/from/1/to/2?format=json&open=true")
    assert resp.status_code == 200
    data = json.loads(resp.text)
    assert all(cs["open"] for cs in data["changesets"])
