"""News Candidate Normalizer and Deduplication Service.

Clean Architecture Layer: Application Business Rules / Domain Services.
Converts raw feed dictionaries to NormalizedCandidate entities.
"""

from __future__ import annotations

import datetime
from typing import Any, Dict, List, Optional, Set, Tuple

from .domain import (
    NormalizedCandidate,
    canonicalize_url,
    compute_content_hash,
    compute_title_hash,
    is_safe_url,
    normalize_publisher,
    parse_iso_utc,
)


class NewsNormalizer:
    """Normalizes raw news items, validates timestamps, and enforces freshness & uniqueness."""

    VERSION: str = "1.0.0"

    def __init__(
        self,
        freshness_window_hours: float = 48.0,
        allowed_domains: Optional[Set[str]] = None,
    ) -> None:
        self.freshness_window_hours = float(freshness_window_hours)
        self.allowed_domains = allowed_domains

    def normalize_item(self, raw_item: Dict[str, Any]) -> NormalizedCandidate:
        """Transform a single raw item dict into a canonical NormalizedCandidate."""
        raw_id = str(raw_item.get("id") or "").strip()
        orig_url = str(raw_item.get("url") or "").strip()
        title = str(raw_item.get("title") or "").strip()
        raw_published = str(raw_item.get("published_at") or "").strip()

        if not raw_id:
            raise ValueError("Raw item missing required 'id'")
        if not orig_url:
            raise ValueError("Raw item missing required 'url'")
        if not title:
            raise ValueError("Raw item missing required 'title'")
        if not raw_published:
            raise ValueError("Raw item missing required 'published_at'")

        if not is_safe_url(orig_url, self.allowed_domains):
            raise ValueError(f"URL violates security boundary / domain filter: {orig_url}")

        canonical_url = canonicalize_url(orig_url)
        norm_pub = normalize_publisher(raw_item.get("publisher"))
        pub_utc = parse_iso_utc(raw_published)

        desc = str(raw_item.get("description") or "").strip() or None
        body = str(raw_item.get("body") or "").strip() or None

        content_h = compute_content_hash(title, body or desc, canonical_url)
        title_h = compute_title_hash(title)
        candidate_id = f"cand_{content_h[:16]}"

        raw_provenance: Dict[str, Any] = {
            "source_id": raw_id,
            "source_feed": str(raw_item.get("source_feed") or "daily-news"),
        }
        if isinstance(raw_item.get("extra"), dict):
            raw_provenance["extra"] = raw_item["extra"]

        tags = [str(t).strip() for t in raw_item.get("tags", []) if str(t).strip()]

        return NormalizedCandidate(
            candidate_id=candidate_id,
            canonical_url=canonical_url,
            original_url=orig_url,
            title=title,
            publisher=str(raw_item.get("publisher") or norm_pub),
            normalized_publisher=norm_pub,
            published_at_utc=pub_utc,
            content_hash=content_h,
            title_hash=title_h,
            raw_provenance=raw_provenance,
            adapter_version=self.VERSION,
            author=raw_item.get("author"),
            description=desc,
            body=body,
            tags=tags,
            language=raw_item.get("language") or "en",
            image_url=raw_item.get("image_url"),
        )

    def ingest_and_normalize(
        self,
        raw_items: List[Dict[str, Any]],
        reference_time: Optional[datetime.datetime] = None,
    ) -> Tuple[List[NormalizedCandidate], List[Dict[str, Any]]]:
        """Normalize multiple raw items, filter staleness/future dates, and deduplicate."""
        if reference_time is None:
            ref = datetime.datetime.now(datetime.timezone.utc)
        elif reference_time.tzinfo is None:
            ref = reference_time.replace(tzinfo=datetime.timezone.utc)
        else:
            ref = reference_time.astimezone(datetime.timezone.utc)

        cutoff = ref - datetime.timedelta(hours=self.freshness_window_hours)

        seen_content_hashes: Set[str] = set()
        seen_title_hashes: Set[str] = set()
        normalized_list: List[NormalizedCandidate] = []
        diagnostics: List[Dict[str, Any]] = []

        for item in raw_items:
            if not isinstance(item, dict):
                diagnostics.append({"reason": "invalid_format", "error": "Item must be a dictionary"})
                continue

            raw_id = str(item.get("id") or "missing_id")
            try:
                candidate = self.normalize_item(item)
            except Exception as e:
                diagnostics.append({"raw_item_id": raw_id, "reason": "normalization_failed", "error": str(e)})
                continue

            item_dt = datetime.datetime.fromisoformat(candidate.published_at_utc)
            if item_dt > ref:
                diagnostics.append({
                    "raw_item_id": raw_id,
                    "reason": "future_published_at",
                    "published_at_utc": candidate.published_at_utc,
                })
                continue

            if item_dt < cutoff:
                diagnostics.append({
                    "raw_item_id": raw_id,
                    "reason": "stale_content",
                    "published_at_utc": candidate.published_at_utc,
                    "cutoff": cutoff.isoformat(),
                })
                continue

            if candidate.content_hash in seen_content_hashes or candidate.title_hash in seen_title_hashes:
                diagnostics.append({
                    "raw_item_id": raw_id,
                    "reason": "duplicate_content",
                    "candidate_id": candidate.candidate_id,
                })
                continue

            seen_content_hashes.add(candidate.content_hash)
            seen_title_hashes.add(candidate.title_hash)
            normalized_list.append(candidate)

        return normalized_list, diagnostics
