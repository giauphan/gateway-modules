"""Feed Fetcher Service for ingesting upstream news items.

Clean Architecture Layer: Interface Adapters / Infrastructure Outbound Port.
Fetches items from official technical specifications, NewsNow API, or file feeds.
"""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import json
import logging
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from .providers.redis_provider import RedisFeedProvider

logger = logging.getLogger(__name__)

DEFAULT_SOURCES = ("hackernews", "github", "producthunt", "reddit", "arxiv")


class FeedFetcher:
    """Fetches raw feeds from official technical sources, NewsNow, or local feeds."""

    def __init__(
        self,
        newsnow_base_url: str = "http://127.0.0.1:4444",
        timeout_seconds: float = 15.0,
        redis_provider: Optional[RedisFeedProvider] = None,
        redis_url: Optional[str] = None,
        redis_key: Optional[str] = None,
        redis_type: Optional[str] = None,
    ) -> None:
        self.newsnow_base_url = newsnow_base_url.rstrip("/")
        self.timeout_seconds = timeout_seconds
        self.base_dir = Path(__file__).resolve().parent
        if redis_provider is not None:
            self.redis_provider = redis_provider
        elif redis_url or redis_key:
            self.redis_provider = RedisFeedProvider(
                redis_url=redis_url or "redis://localhost:6379/0",
                key=redis_key or "news:incoming",
                channel_type=redis_type or "stream",
                timeout_seconds=min(3.0, timeout_seconds),
            )
        else:
            self.redis_provider = None

    @staticmethod
    def _extract_published_at(raw_item: Dict[str, Any], payload: Dict[str, Any]) -> Optional[str]:
        """Extract reliable published timestamp from feed record."""
        for field in ("published_at", "publishedAt", "pubDate", "date", "created_at"):
            val = raw_item.get(field)
            if isinstance(val, str) and val.strip():
                return val.strip()
        return datetime.now(timezone.utc).isoformat()

    @staticmethod
    def _host(url: str) -> str:
        return urlparse(url).hostname or "newsnow"

    def fetch_redis_feed(self, limit: int = 15) -> List[Dict[str, Any]]:
        """Fetch items directly from configured Redis provider."""
        if self.redis_provider:
            return self.redis_provider.fetch_items(limit=limit)
        return []

    def fetch_official_sources(self) -> List[Dict[str, Any]]:
        """Fetch verified official technical sources (MCP, Anthropic, GitHub, Git, SWE-bench)."""
        candidates: List[Dict[str, Any]] = []

        # 1. Bundled official sources
        bundled_path = self.base_dir / "data" / "official_sources.json"
        if bundled_path.is_file():
            candidates.extend(self.fetch_from_file(bundled_path))

        # 2. OpenMontage production feed fallback if available
        om_feed = Path("/home/ubuntu/OpenMontage/config/sources/production_agent_engineering_feed.json")
        if not candidates and om_feed.is_file():
            candidates.extend(self.fetch_from_file(om_feed))

        return candidates

    def fetch_newsnow_feed(
        self,
        sources: Sequence[str] = DEFAULT_SOURCES,
    ) -> List[Dict[str, Any]]:
        """Fetch items across multiple NewsNow sources and optional Redis stream."""
        items: List[Dict[str, Any]] = []
        seen_urls: set[str] = set()

        if "redis" in sources and self.redis_provider:
            redis_items = self.fetch_redis_feed(limit=15)
            for r_item in redis_items:
                u_key = str(r_item.get("url") or "").lower().rstrip("/")
                if u_key:
                    seen_urls.add(u_key)
                items.append(r_item)

        active_sources = [s for s in sources if s != "redis"]
        for source in active_sources:
            endpoint = f"{self.newsnow_base_url}/api/s?id={source}"
            try:
                req = Request(endpoint, headers={"Accept": "application/json", "User-Agent": "toolcase-daily-news/1.0"})
                with urlopen(req, timeout=self.timeout_seconds) as resp:
                    payload = json.load(resp)
            except Exception as e:
                logger.warning("Failed fetching NewsNow source %s from %s: %s", source, endpoint, e)
                continue

            if not isinstance(payload, dict) or not isinstance(payload.get("items"), list):
                continue

            for raw in payload["items"]:
                if not isinstance(raw, dict):
                    continue
                url = str(raw.get("url") or "").strip()
                title = str(raw.get("title") or "").strip()
                if not url or not title:
                    continue
                norm_key = url.lower().rstrip("/")
                if norm_key in seen_urls:
                    continue
                seen_urls.add(norm_key)

                published_at = self._extract_published_at(raw, payload)
                extra = raw.get("extra") if isinstance(raw.get("extra"), dict) else {}
                extra = {**extra, "retrieved_at": datetime.now(timezone.utc).isoformat()}
                info = str(extra.get("info") or raw.get("description") or "").strip()
                item_id = str(raw.get("id") or norm_key)

                item = {
                    "id": f"newsnow_{source}_{item_id}",
                    "url": url,
                    "title": title,
                    "publisher": self._host(url),
                    "published_at": published_at,
                    "description": info,
                    "body": info,
                    "tags": [source],
                    "language": "en",
                    "source_feed": f"newsnow:{source}",
                    "extra": extra,
                }
                items.append(item)

        # Fallback to direct public HackerNews API if local NewsNow daemon yielded 0 items
        if not items and ("hackernews" in sources or "all" in sources):
            direct_items = self.fetch_hackernews_direct(limit=15)
            if direct_items:
                logger.info("Retrieved %d live items via direct HackerNews fallback", len(direct_items))
                items.extend(direct_items)

        return items

    def fetch_hackernews_direct(self, limit: int = 15) -> List[Dict[str, Any]]:
        """Direct fallback fetching top stories from HackerNews Firebase API."""
        endpoint = "https://hacker-news.firebaseio.com/v0/topstories.json"
        try:
            req = Request(endpoint, headers={"Accept": "application/json", "User-Agent": "toolcase-daily-news/1.0"})
            with urlopen(req, timeout=4.0) as resp:
                ids = json.load(resp)
            if not isinstance(ids, list):
                return []
            target_ids = ids[:max(1, limit)]
        except Exception as e:
            logger.warning("Failed querying HackerNews top stories: %s", e)
            return []

        def fetch_item(item_id: int | str) -> Optional[Dict[str, Any]]:
            item_url = f"https://hacker-news.firebaseio.com/v0/item/{item_id}.json"
            try:
                r = Request(item_url, headers={"Accept": "application/json", "User-Agent": "toolcase-daily-news/1.0"})
                with urlopen(r, timeout=3.0) as resp:
                    return json.load(resp)
            except Exception:
                return None

        with ThreadPoolExecutor(max_workers=5) as pool:
            raw_items = [it for it in pool.map(fetch_item, target_ids) if isinstance(it, dict)]

        items: List[Dict[str, Any]] = []
        for raw in raw_items:
            title = str(raw.get("title") or "").strip()
            item_id = str(raw.get("id") or "")
            url = str(raw.get("url") or f"https://news.ycombinator.com/item?id={item_id}").strip()
            if not title or not item_id:
                continue

            unix_ts = raw.get("time")
            pub_iso = (
                datetime.fromtimestamp(unix_ts, tz=timezone.utc).isoformat()
                if unix_ts
                else datetime.now(timezone.utc).isoformat()
            )
            info = str(raw.get("text") or "").strip()

            items.append({
                "id": f"hn_{item_id}",
                "url": url,
                "title": title,
                "publisher": self._host(url),
                "published_at": pub_iso,
                "description": info,
                "body": info,
                "tags": ["hackernews", "tech"],
                "language": "en",
                "source_feed": "live:hackernews",
                "extra": {
                    "score": raw.get("score"),
                    "by": raw.get("by"),
                    "comments": raw.get("descendants"),
                },
            })
        return items

    def fetch_from_file(self, file_path: str | Path) -> List[Dict[str, Any]]:
        """Load items from a local JSON feed file."""
        p = Path(file_path)
        if not p.is_file():
            return []
        try:
            with p.open("r", encoding="utf-8") as f:
                data = json.load(f)
            if isinstance(data, list):
                return data
            if isinstance(data, dict) and isinstance(data.get("items"), list):
                return data["items"]
            return []
        except Exception as e:
            logger.warning("Failed reading feed file %s: %s", file_path, e)
            return []
