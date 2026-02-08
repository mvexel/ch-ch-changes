"""Configuration via environment variables."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional

from pydantic import field_validator
from pydantic_settings import BaseSettings


class ServerConfig(BaseSettings):
    """Server configuration loaded from environment variables."""

    model_config = {"env_prefix": "", "case_sensitive": True}

    DATA_PATH: str = "./data"
    PORT: int = 3000
    HOST: str = "0.0.0.0"

    # Auth
    REQUIRE_AUTH: bool = True
    OSM_OAUTH_CLIENT_ID: Optional[str] = None
    OSM_OAUTH_CLIENT_SECRET: Optional[str] = None
    OSM_OAUTH_REDIRECT_URL: Optional[str] = None
    OSM_OAUTH_AUTHORIZE_URL: str = "https://www.openstreetmap.org/oauth2/authorize"
    OSM_OAUTH_TOKEN_URL: str = "https://www.openstreetmap.org/oauth2/token"
    OSM_OAUTH_USER_DETAILS_URL: str = "https://api.openstreetmap.org/api/0.6/user/details.json"
    OSM_OAUTH_SCOPE: str = "read_prefs"
    AUTH_KEY_SECRET: Optional[str] = None
    AUTH_DB_PATH: Optional[str] = None
    AUTH_DEFAULT_DAILY_LIMIT: Optional[int] = None
    AUTH_DEFAULT_MONTHLY_LIMIT: Optional[int] = None

    def get_auth_db_path(self) -> Path:
        if self.AUTH_DB_PATH:
            return Path(self.AUTH_DB_PATH)
        home = os.environ.get("HOME", ".")
        return Path(home) / "changeset-auth.sqlite"


class WorkerConfig(BaseSettings):
    """Worker configuration loaded from environment variables."""

    model_config = {"env_prefix": "", "case_sensitive": True}

    DATA_PATH: str = "/opt/changeset-data/changesets"
    DELTA_DIR: Optional[str] = None
    REPLICATION_STATE_FILE: Optional[str] = None
    REPLICATION_TMP_DIR: Optional[str] = None
    CHANGESET_REPLICATION_URL: str = "https://planet.openstreetmap.org/replication/changesets/"
    REPLICATION_START_SEQUENCE: Optional[int] = None
    REPLICATION_START_TIMESTAMP: Optional[str] = None

    @field_validator("REPLICATION_START_SEQUENCE", mode="before")
    @classmethod
    def empty_str_to_none(cls, v):
        """Convert empty strings to None for optional int fields."""
        if v == "" or v is None:
            return None
        return v

    def get_delta_dir(self) -> Path:
        if self.DELTA_DIR:
            return Path(self.DELTA_DIR)
        return Path(self.DATA_PATH) / "delta"

    def get_state_file(self) -> Path:
        if self.REPLICATION_STATE_FILE:
            return Path(self.REPLICATION_STATE_FILE)
        return self.get_delta_dir() / "state.yaml"

    def get_tmp_dir(self) -> Path:
        if self.REPLICATION_TMP_DIR:
            return Path(self.REPLICATION_TMP_DIR)
        return self.get_delta_dir() / "tmp"
