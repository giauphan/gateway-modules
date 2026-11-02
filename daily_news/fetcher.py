"""Feed Fetcher Service for ingesting upstream news items.

Clean Architecture Layer: Interface Adapters / Infrastructure Outbound Port.
Fetches items from NewsNow API or configured upstream feeds.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import logging
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence
from urllib.parse import urlparse
from urllib.request import Request, urlopen

logger = logging.getLogger(__name__)

DEFAULT_SOURCES = ("hackernews", "github", "producthunt")


class FeedFetcher:
    """Fetches raw feeds from upstream NewsNow service or local feed sources."""

    def __init__(
        self,
        newsnow_base_url: str = "http://127.0.0.1:4444",
        timeout_seconds: float = 15.0,
    ) -> None:
        self.newsnow_base_url = newsnow_base_url.rstrip("/")
        self.timeout_seconds = timeout_seconds

    @staticmethod
    def _extract_published_at(raw_item: Dict[str, Any], payload: Dict[str, Any]) -> Optional[str]:
        """Extract reliable published timestamp from feed record."""
        for field in ("published_at", "publishedAt", "pubDate", "date", "created_at"):
            val = raw_item.get(field)
            if isinstance(val, str) and val.strip():
                return val.strip()
        # Fallback to current time if feed does not supply one
        return datetime.now(timezone.utc).isoformat()

    @staticmethod
    def _host(url: str) -> str:
        return urlparse(url).hostname or "newsnow"

    def fetch_newsnow_feed(
        self,
        sources: Sequence[str] = DEFAULT_SOURCES,
    ) -> List[Dict[str, Any]]:
        """Fetch items across multiple NewsNow sources."""
        items: List[Dict[str, Any]] = []
        seen_urls: set[str] = set()

        for source in sources:
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
