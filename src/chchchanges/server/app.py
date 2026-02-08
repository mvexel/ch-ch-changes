"""FastAPI application setup and entry point."""

from __future__ import annotations

import logging
import time
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from chchchanges.config import ServerConfig
from chchchanges.server.query import QueryEngine
from chchchanges.server.routes import auth, changesets, meta

logger = logging.getLogger(__name__)


def create_app(config: ServerConfig | None = None) -> FastAPI:
    """Create and configure the FastAPI application."""
    if config is None:
        config = ServerConfig()

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        logger.info("Starting chchchanges server, data_path=%s", config.DATA_PATH)
        app.state.engine = QueryEngine(config.DATA_PATH)
        app.state.started_at = time.time()
        app.state.config = config
        yield

    app = FastAPI(
        title="chchchanges",
        description="OSM Changeset API backed by DuckDB/Parquet",
        version="0.1.0",
        lifespan=lifespan,
    )

    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_methods=["*"],
        allow_headers=["*"],
    )

    app.include_router(meta.router)
    app.include_router(changesets.router)
    app.include_router(auth.router)

    # Serve static files (landing page + assets)
    static_dir = Path(__file__).parent.parent.parent.parent / "static"
    if static_dir.exists():
        app.mount("/static", StaticFiles(directory=str(static_dir)), name="static")

        @app.get("/", include_in_schema=False)
        async def root():
            """Serve the landing page."""
            return FileResponse(static_dir / "index.html")

    return app


def main():
    """CLI entry point for chch-server."""
    import uvicorn

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    config = ServerConfig()
    app = create_app(config)
    uvicorn.run(app, host=config.HOST, port=config.PORT)


if __name__ == "__main__":
    main()
