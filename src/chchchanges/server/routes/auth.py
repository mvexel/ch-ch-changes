"""Auth routes — stubbed for REQUIRE_AUTH=false."""

from __future__ import annotations

from fastapi import APIRouter

router = APIRouter(prefix="/auth", tags=["Auth"])


@router.get("/osm/start")
async def oauth_start():
    return {"error": "Authentication is not enabled. Set REQUIRE_AUTH=true to enable."}


@router.get("/osm/callback")
async def oauth_callback():
    return {"error": "Authentication is not enabled. Set REQUIRE_AUTH=true to enable."}


@router.get("/me")
async def get_me():
    return {"error": "Authentication is not enabled. Set REQUIRE_AUTH=true to enable."}
