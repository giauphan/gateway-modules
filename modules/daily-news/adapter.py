#!/usr/bin/env python3
"""Daily News & Content Discovery Service Adapter for Toolcase Gateway.

Protocol: stdio-stream (JSON Lines over stdin / stdout).
Clean Architecture Layer: Interface Adapters.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import time
from typing import Any, Dict, List
import uuid

from daily_news import (
    CandidateScorer,
    FeedFetcher,
    NewsNormalizer,
    NewsStateStore,
    TieredContentDiscovery,
    render_dashboard_html,
)

MODULE_ID = os.environ.get("MODULE_ID", "daily-news")
MODULE_NAME = os.environ.get("MODULE_NAME", "Daily News & Content Discovery Service")

BASE_DIR = Path(__file__).resolve().parent
STATE_DIR = Path(os.environ.get("GW_STATE_DIR", "/tmp/gw_daily_news_state"))
store = NewsStateStore(STATE_DIR)

DEFAULT_CONFIG: Dict[str, Any] = {
    "name": MODULE_NAME,
    "newsnow_url": os.environ.get("NEWSNOW_API_URL", "http://127.0.0.1:4444"),
    "freshness_window_hours": float(os.environ.get("FRESHNESS_WINDOW_HOURS", "48.0")),
    "max_age_days": int(os.environ.get("NEWS_MAX_AGE_DAYS", "7")),
    "require_latest_news": True,
    "default_sources": ["hackernews", "github", "producthunt", "reddit", "arxiv"],
}


def get_active_config() -> Dict[str, Any]:
    persisted = store.load_config()
    cfg = {**DEFAULT_CONFIG, **persisted}
    return cfg


def json_resp(status: int, payload: Any) -> Dict[str, Any]:
    return {
        "status": status,
        "headers": {"content-type": "application/json"},
        "body": json.dumps(payload, ensure_ascii=False),
    }


def html_resp(status: int, content: str) -> Dict[str, Any]:
    return {
        "status": status,
        "headers": {"content-type": "text/html; charset=utf-8"},
        "body": content,
    }


def handle_request(req: Dict[str, Any]) -> Dict[str, Any]:
    path = str(req.get("path") or "").rstrip("/")
    method = str(req.get("method") or "GET").upper()
    raw_body = req.get("body") or ""

    body: Dict[str, Any] = {}
    if raw_body and isinstance(raw_body, str):
        try:
            body = json.loads(raw_body)
        except Exception:
            body = {}
    elif isinstance(raw_body, dict):
        body = raw_body

    cfg = get_active_config()
    fetcher = FeedFetcher(newsnow_base_url=cfg["newsnow_url"])
    normalizer = NewsNormalizer(freshness_window_hours=cfg["freshness_window_hours"])
    scorer = CandidateScorer()
    discovery = TieredContentDiscovery(
        store=store,
        fetcher=fetcher,
        normalizer=normalizer,
        scorer=scorer,
        max_age_days=cfg["max_age_days"],
        require_latest_news=cfg["require_latest_news"],
    )

    # 1. UI / Root Dashboard
    if path in ("", "/ui") or path.endswith(("/custom/daily-news", "/custom/daily-news/ui")):
        candidates = store.get_cached_candidates()
        stats = {
            "consumed_count": len(store.get_consumed_ids()),
        }
        return html_resp(200, render_dashboard_html(cfg, stats, candidates))

    # 2. Health Check
    if path.endswith(("/health", "/v1/health")):
        return json_resp(200, {
            "status": "healthy",
            "module": MODULE_ID,
            "version": "1.0.0",
            "cached_candidates": len(store.get_cached_candidates()),
            "consumed_count": len(store.get_consumed_ids()),
            "newsnow_url": cfg["newsnow_url"],
        })

    # 3. OpenAI Models
    if path.endswith("/models"):
        return json_resp(200, {
            "object": "list",
            "data": [
                {
                    "id": MODULE_ID,
                    "object": "model",
                    "created": int(time.time()),
                    "owned_by": "toolcase",
                }
            ],
        })

    # 4. Chat Completions (News Summary)
    if path.endswith("/chat/completions"):
        limit = body.get("limit", 3)
        res = discovery.discover_candidates(count=limit)
        items = res.candidates
        if items:
            lines = [f"### Daily News Briefing ({len(items)} items):\n"]
            for i, item in enumerate(items, 1):
                t = item.get("title", "")
                u = item.get("canonical_url", "")
                p = item.get("publisher", "")
                s = item.get("scoring", {}).get("total_score", 0.0)
                lines.append(f"{i}. **{t}** ({p}) [Score: {s:.2f}]\n   {u}")
            reply = "\n".join(lines)
        else:
            reply = "No eligible news candidates found within the current freshness window."

        resp = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": MODULE_ID,
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": reply},
                    "finish_reason": "stop",
                }
            ],
            "usage": {"prompt_tokens": 10, "completion_tokens": len(reply.split()), "total_tokens": 50},
        }
        return json_resp(200, resp)

    # 5. Raw Feed Endpoint
    if path.endswith("/api/v1/feed"):
        sources = body.get("sources") or cfg.get("default_sources", ["hackernews", "github", "producthunt"])
        raw = fetcher.fetch_newsnow_feed(sources=sources)
        return json_resp(200, {"count": len(raw), "items": raw})

    # 6. Normalized & Ranked Candidates Endpoint
    if path.endswith("/api/v1/candidates"):
        if method == "POST" and "items" in body:
            raw_items = body["items"]
        else:
            sources = cfg.get("default_sources", ["hackernews", "github", "producthunt"])
            raw_items = fetcher.fetch_newsnow_feed(sources=sources)

        normalized, diagnostics = normalizer.ingest_and_normalize(raw_items)
        limit = int(body.get("limit") or 10)
        ranked = scorer.rank_candidates(normalized, limit=limit)
        return json_resp(200, {
            "count": len(ranked),
            "candidates": ranked,
            "dropped_count": len(diagnostics),
            "diagnostics": diagnostics[:20],
        })

    # 7. Tiered Discovery Endpoint
    if path.endswith("/api/v1/discover"):
        count = int(body.get("count") or 5)
        curated = body.get("curated_candidates")
        sources = body.get("sources")
        official_only = bool(body.get("official_only", False) or (isinstance(sources, list) and sources == ["official"]))
        res = discovery.discover_candidates(
            count=count,
            curated_feed=curated,
            feed_sources=sources,
            official_only=official_only,
        )
        return json_resp(200, res.to_dict())

    # 8. Mark Consumed Endpoint
    if path.endswith("/api/v1/consume") and method == "POST":
        ids = body.get("candidate_ids") or []
        if isinstance(ids, list):
            store.mark_consumed(ids)
            return json_resp(200, {"status": "ok", "consumed_count": len(store.get_consumed_ids())})
        return json_resp(400, {"error": "candidate_ids must be a list of strings"})

    # 9. Config Get/Set
    if path.endswith("/api/v1/config"):
        if method == "POST":
            for k in ("newsnow_url", "freshness_window_hours", "max_age_days", "require_latest_news"):
                if k in body:
                    cfg[k] = body[k]
            store.save_config(cfg)
            return json_resp(200, {"status": "updated", "config": cfg})
        return json_resp(200, cfg)

    # 404 Fallback
    return json_resp(404, {"error": f"Route not found: {path}"})


def main() -> None:
    """Read JSON lines from stdin, dispatch, and write JSON responses to stdout."""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            out = handle_request(req)
        except Exception as e:
            out = json_resp(500, {"error": str(e)})

        sys.stdout.write(json.dumps(out, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
