"""Tiered Content Discovery Service implementing Priority Cascading.

Clean Architecture Layer: Application Business Rules (Use Cases).
Tier 1 (Curated Queue & Official Specs) -> Tier 2 (Automated Live Feeds) -> Tier 3 (Fail-Closed).
Depends strictly on abstract Outbound Ports via typing.Protocol (DIP).
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import logging
from typing import Any, Dict, List, Optional, Sequence, Set

from .domain import DiscoveryResult, NormalizedCandidate
from .fetcher import DEFAULT_SOURCES, FeedFetcher
from .normalizer import NewsNormalizer
from .ports import (
    CandidateScorerPort,
    FeedFetcherPort,
    NewsNormalizerPort,
    NewsStateStorePort,
)
from .scoring import CandidateScorer

logger = logging.getLogger(__name__)


class TieredContentDiscovery:
    """Orchestrates tiered news and content discovery with deduplication and fail-closed safety."""

    def __init__(
        self,
        store: NewsStateStorePort,
        fetcher: Optional[FeedFetcherPort] = None,
        normalizer: Optional[NewsNormalizerPort] = None,
        scorer: Optional[CandidateScorerPort] = None,
        max_age_days: int = 7,
        require_latest_news: bool = True,
    ) -> None:
        self.store = store
        self.fetcher = fetcher or FeedFetcher()
        self.normalizer = normalizer or NewsNormalizer()
        self.scorer = scorer or CandidateScorer()
        self.max_age_days = max(1, int(max_age_days))
        self.require_latest_news = require_latest_news

    def _filter_and_score(
        self,
        raw_items: List[Dict[str, Any]],
        consumed_ids: Set[str],
        reference_time: datetime,
        limit: int,
    ) -> List[Dict[str, Any]]:
        normalized, _ = self.normalizer.ingest_and_normalize(raw_items, reference_time)
        unconsumed: List[NormalizedCandidate] = [
            c for c in normalized if c.candidate_id not in consumed_ids
        ]

        if not unconsumed:
            return []

        if self.require_latest_news:
            unconsumed.sort(key=lambda c: c.published_at_utc, reverse=True)

        ranked = self.scorer.rank_candidates(unconsumed, limit=limit)
        return ranked

    def discover_candidates(
        self,
        count: int = 5,
        curated_feed: Optional[List[Dict[str, Any]]] = None,
        feed_sources: Optional[Sequence[str]] = None,
        official_only: bool = False,
    ) -> DiscoveryResult:
        """Execute tiered discovery with fallback and deterministic outcome."""
        ref_time = datetime.now(timezone.utc)
        consumed_ids = self.store.get_consumed_ids()
        diagnostics: Dict[str, Any] = {
            "consumed_ids_count": len(consumed_ids),
            "tier_1_checked": False,
            "tier_2_checked": False,
            "official_only": official_only,
            "require_latest_news": self.require_latest_news,
            "max_age_days": self.max_age_days,
            "reference_time": ref_time.isoformat(),
        }

        # --- Tier 1: Curated Backlog & Official Sources ---
        diagnostics["tier_1_checked"] = True
        official_pool: List[Dict[str, Any]] = []
        is_curated_custom = False

        if curated_feed is not None and len(curated_feed) > 0:
            official_pool.extend(curated_feed)
            is_curated_custom = True
        elif official_only or feed_sources is None or "official" in feed_sources or "all" in feed_sources:
            official_pool.extend(self.fetcher.fetch_official_sources())

        if official_pool:
            t1_ranked = self._filter_and_score(official_pool, consumed_ids, ref_time, count)
            diagnostics["tier_1_valid_count"] = len(t1_ranked)
            if t1_ranked:
                self.store.save_cached_candidates(t1_ranked)
                tier_name = "tier_1_curated" if is_curated_custom else "tier_1_curated_official"
                return DiscoveryResult(
                    tier=tier_name,
                    status="ELIGIBLE",
                    candidates=t1_ranked,
                    source_file="curated_feed" if is_curated_custom else "official_sources",
                    diagnostics=diagnostics,
                )

        if official_only:
            return DiscoveryResult(
                tier="none",
                status="NO_ELIGIBLE_TOPIC",
                candidates=[],
                reason="No unconsumed candidates found in official documentation sources",
                diagnostics=diagnostics,
            )

        # --- Tier 2: Automated Live Feed (HackerNews, GitHub, ProductHunt, Reddit, ArXiv) ---
        diagnostics["tier_2_checked"] = True
        active_sources = [s for s in (feed_sources or DEFAULT_SOURCES) if s != "official"]
        if not active_sources:
            active_sources = list(DEFAULT_SOURCES)

        live_raw = self.fetcher.fetch_newsnow_feed(sources=active_sources)
        diagnostics["tier_2_raw_count"] = len(live_raw)

        if live_raw:
            t2_ranked = self._filter_and_score(live_raw, consumed_ids, ref_time, count)
            diagnostics["tier_2_valid_count"] = len(t2_ranked)
            if t2_ranked:
                self.store.save_cached_candidates(t2_ranked)
                return DiscoveryResult(
                    tier="tier_2_automated",
                    status="ELIGIBLE",
                    candidates=t2_ranked,
                    source_file="live_newsnow_feed",
                    diagnostics=diagnostics,
                )

        # --- Fallback: Cached candidates ---
        cached = self.store.get_cached_candidates()
        cached_unconsumed = [c for c in cached if c.get("candidate_id") not in consumed_ids]
        if cached_unconsumed:
            return DiscoveryResult(
                tier="tier_2_cache",
                status="ELIGIBLE",
                candidates=cached_unconsumed[:count],
                source_file="cached_candidates",
                diagnostics=diagnostics,
            )

        # --- Tier 3: Clean Fail-Closed ---
        return DiscoveryResult(
            tier="none",
            status="NO_ELIGIBLE_TOPIC",
            candidates=[],
            reason="Zero candidates passed freshness, deduplication, and scope boundaries",
            diagnostics=diagnostics,
        )
