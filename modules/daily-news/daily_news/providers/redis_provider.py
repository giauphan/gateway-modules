"""Redis Queue/Stream Feed Provider for technical news ingestion.

Clean Architecture Layer: Interface Adapters / Infrastructure Provider.
Connects synchronously to Redis Streams (XREVRANGE), Lists (LRANGE), or Sorted Sets.
Encapsulates all redis-py operations, error handling, and timeout safeguards.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import logging
from typing import Any, Dict, List, Optional

logger = logging.getLogger(__name__)


class RedisFeedProvider:
    """Ingests raw candidate news articles from Redis Streams or Queues."""

    def __init__(
        self,
        redis_url: str = "redis://localhost:6379/0",
        key: str = "news:incoming",
        channel_type: str = "stream",
        timeout_seconds: float = 3.0,
        client: Optional[Any] = None,
    ) -> None:
        self.redis_url = redis_url
        self.key = key.strip() if key else "news:incoming"
        self.channel_type = (channel_type or "stream").strip().lower()
        self.timeout_seconds = max(0.5, float(timeout_seconds))
        self._client = client

    def _get_client(self) -> Any:
        if self._client is not None:
            return self._client
        try:
            import redis

            self._client = redis.Redis.from_url(
                self.redis_url,
                socket_timeout=self.timeout_seconds,
                socket_connect_timeout=self.timeout_seconds,
                decode_responses=True,
            )
            return self._client
        except Exception as e:
            logger.warning("Failed initializing Redis client for %s: %s", self.redis_url, e)
            return None

    @staticmethod
    def _timestamp_from_msg_id(msg_id: str) -> Optional[str]:
        """Extract UTC ISO-8601 timestamp from Redis Stream ID (<timestamp_ms>-<seq>)."""
        try:
            prefix = msg_id.split("-")[0]
            ts_sec = int(prefix) / 1000.0
            return datetime.fromtimestamp(ts_sec, tz=timezone.utc).isoformat()
        except Exception:
            return None

    def _normalize_payload(self, raw_data: Any, msg_id: Optional[str] = None) -> Optional[Dict[str, Any]]:
        """Map raw dictionary or serialized JSON string into standardized raw news item."""
        if not raw_data:
            return None

        payload: Dict[str, Any] = {}
        if isinstance(raw_data, str):
            try:
                payload = json.loads(raw_data)
            except Exception:
                return None
        elif isinstance(raw_data, dict):
            if "payload" in raw_data and isinstance(raw_data["payload"], str):
                try:
                    payload = json.loads(raw_data["payload"])
                except Exception:
                    payload = raw_data
            elif "json" in raw_data and isinstance(raw_data["json"], str):
                try:
                    payload = json.loads(raw_data["json"])
                except Exception:
                    payload = raw_data
            else:
                payload = raw_data

        if not isinstance(payload, dict):
            return None

        title = str(payload.get("title") or "").strip()
        url = str(payload.get("url") or payload.get("link") or "").strip()
        if not title and not url:
            return None

        item_id = str(payload.get("id") or (f"redis_{msg_id}" if msg_id else f"redis_{hash(url)}"))
        if not url:
            url = f"redis://{self.key}/{item_id}"

        published_at = (
            payload.get("published_at")
            or payload.get("published_at_utc")
            or payload.get("created_at")
            or (self._timestamp_from_msg_id(msg_id) if msg_id else None)
            or datetime.now(timezone.utc).isoformat()
        )

        body = str(payload.get("body") or payload.get("description") or payload.get("summary") or "").strip()
        tags = payload.get("tags")
        if isinstance(tags, str):
            tags = [t.strip() for t in tags.split(",") if t.strip()]
        elif not isinstance(tags, list):
            tags = ["redis", self.channel_type]

        item: Dict[str, Any] = {
            "id": item_id,
            "url": url,
            "title": title or f"Redis Update {item_id}",
            "publisher": str(payload.get("publisher") or "Redis Ingest").strip(),
            "published_at": str(published_at).strip(),
            "description": body,
            "body": body,
            "tags": tags,
            "language": str(payload.get("language") or "en"),
            "source_feed": f"redis:{self.key}",
            "extra": {
                "channel_type": self.channel_type,
                "redis_key": self.key,
                "msg_id": msg_id,
            },
        }

        # Pass-through 6-beat storyboard fields if already provided by upstream crawler
        for beat_field in (
            "symptom",
            "root_cause",
            "remedy",
            "hook",
            "viewer_question",
            "evidence_excerpt",
            "source_snapshot",
            "snapshot_sha256",
            "topic_family",
        ):
            if beat_field in payload:
                item[beat_field] = payload[beat_field]

        return item

    def fetch_items(self, limit: int = 15) -> List[Dict[str, Any]]:
        """Fetch latest raw items from configured Redis stream, list, or sorted set."""
        client = self._get_client()
        if client is None:
            return []

        limit = max(1, int(limit))
        items: List[Dict[str, Any]] = []

        try:
            if self.channel_type == "stream":
                # XREVRANGE key + - COUNT limit (reads newest to oldest)
                entries = client.xrevrange(self.key, max="+", min="-", count=limit)
                for msg_id, fields in entries:
                    parsed = self._normalize_payload(fields, msg_id=str(msg_id))
                    if parsed:
                        items.append(parsed)

            elif self.channel_type == "list":
                # LRANGE key 0 limit-1 (reads newest items from head of FIFO/LIFO list)
                entries = client.lrange(self.key, 0, limit - 1)
                for idx, entry in enumerate(entries):
                    parsed = self._normalize_payload(entry, msg_id=f"idx_{idx}")
                    if parsed:
                        items.append(parsed)

            elif self.channel_type == "zset":
                # ZREVRANGE key 0 limit-1
                entries = client.zrevrange(self.key, 0, limit - 1)
                for idx, entry in enumerate(entries):
                    parsed = self._normalize_payload(entry, msg_id=f"z_{idx}")
                    if parsed:
                        items.append(parsed)

            else:
                logger.warning("Unsupported Redis channel_type '%s'; expected 'stream', 'list', or 'zset'", self.channel_type)

        except Exception as e:
            logger.warning("Failed querying Redis (%s, key=%s): %s", self.redis_url, self.key, e)
            return []

        return items
