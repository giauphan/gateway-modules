"""Tiered Content Discovery Service implementing Priority Cascading.

Clean Architecture Layer: Application Business Rules (Use Cases).
Tier 1 (Curated Queue & Official Specs) -> Tier 2 (Automated Live Feeds) -> Tier 3 (Fail-Closed).
Depends strictly on abstract Outbound Ports via typing.Protocol (DIP).
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import logging
from typing import Any, Dict, List, Optional, Sequence, Set

from .domain import DiscoveryResult, NormalizedCandidate, parse_boundary_datetime
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
        self.max_age_days = max(1, int(max_age_days))
        self.require_latest_news = require_latest_news
        self.fetcher = fetcher or FeedFetcher()
        self.normalizer = normalizer or NewsNormalizer(
            freshness_window_hours=float(self.max_age_days * 24),
            require_latest_news=self.require_latest_news,
        )
        self.scorer = scorer or CandidateScorer()

    def _filter_and_score(
        self,
        raw_items: List[Dict[str, Any]],
        consumed_ids: Set[str],
        reference_time: datetime,
        limit: int,
        topics: Optional[Sequence[str]] = None,
        from_date: Optional[datetime] = None,
        to_date: Optional[datetime] = None,
    ) -> List[Dict[str, Any]]:
        normalized, _ = self.normalizer.ingest_and_normalize(
            raw_items,
            reference_time=reference_time,
            require_latest_news=self.require_latest_news,
            from_date=from_date,
            to_date=to_date,
        )
        unconsumed: List[NormalizedCandidate] = [
            c for c in normalized if c.candidate_id not in consumed_ids
        ]

        if topics:
            targets = {t.strip().lower() for t in topics if t and str(t).strip()}
            if targets:
                unconsumed = [
                    c for c in unconsumed
                    if (
                        c.topic_family.lower() in targets
                        or any(tag.lower() in targets for tag in c.tags)
                        or any(tgt in c.topic_family.lower() for tgt in targets)
                        or any(tgt in c.title.lower() for tgt in targets)
                    )
                ]

        if not unconsumed:
            return []

        if self.require_latest_news:
            unconsumed.sort(
                key=lambda c: c.updated_at or c.published_at_utc,
                reverse=True,
            )

        ranked = self.scorer.rank_candidates(unconsumed, limit=limit)
        return ranked

    def discover_candidates(
        self,
        count: int = 5,
        curated_feed: Optional[List[Dict[str, Any]]] = None,
        feed_sources: Optional[Sequence[str]] = None,
        official_only: bool = False,
        topics: Optional[Sequence[str]] = None,
        from_date: Optional[Any] = None,
        to_date: Optional[Any] = None,
    ) -> DiscoveryResult:
        """Execute tiered discovery with fallback, deterministic outcome, topic and date-range filtering."""
        ref_time = datetime.now(timezone.utc)
        consumed_ids = self.store.get_consumed_ids()
        norm_topics = [str(t).strip() for t in topics if str(t).strip()] if topics else None

        try:
            from_dt = parse_boundary_datetime(from_date, is_end_of_day=False)
            to_dt = parse_boundary_datetime(to_date, is_end_of_day=True)
        except ValueError as err:
            return DiscoveryResult(
                tier="none",
                status="INVALID_DATE_FORMAT",
                candidates=[],
                reason=f"Failed to parse date boundary: {err}",
                diagnostics={"error": str(err)},
            )

        if from_dt and to_dt and from_dt > to_dt:
            return DiscoveryResult(
                tier="none",
                status="INVALID_DATE_RANGE",
                candidates=[],
                reason=f"from_date ({from_dt.isoformat()}) must not be later than to_date ({to_dt.isoformat()})",
                diagnostics={
                    "from_date": from_dt.isoformat(),
                    "to_date": to_dt.isoformat(),
                },
            )

        diagnostics: Dict[str, Any] = {
            "consumed_ids_count": len(consumed_ids),
            "tier_1_checked": False,
            "tier_2_checked": False,
            "official_only": official_only,
            "topics_filter": norm_topics,
            "from_date": from_dt.isoformat() if from_dt else None,
            "to_date": to_dt.isoformat() if to_dt else None,
            "require_latest_news": self.require_latest_news,
            "max_age_days": self.max_age_days,
            "reference_time": ref_time.isoformat(),
        }

        # --- Tier 1 Explicit: Caller-provided curated feed OR explicit official_only request ---
        if curated_feed is not None and len(curated_feed) > 0:
            diagnostics["tier_1_checked"] = True
            t1_ranked = self._filter_and_score(
                curated_feed,
                consumed_ids,
                ref_time,
                count,
                topics=norm_topics,
                from_date=from_dt,
                to_date=to_dt,
            )
            diagnostics["tier_1_valid_count"] = len(t1_ranked)
            if t1_ranked:
                self.store.save_cached_candidates(t1_ranked)
                return DiscoveryResult(
                    tier="tier_1_curated",
                    status="ELIGIBLE",
                    candidates=t1_ranked,
                    source_file="curated_feed",
                    diagnostics=diagnostics,
                )
            return DiscoveryResult(
                tier="none",
                status="NO_ELIGIBLE_TOPIC",
                candidates=[],
                reason="All candidates in curated feed were consumed, stale, or dropped by topic/date filter",
                diagnostics=diagnostics,
            )

        if official_only or (feed_sources is not None and list(feed_sources) == ["official"]):
            diagnostics["tier_1_checked"] = True
            official_pool = self.fetcher.fetch_official_sources()
            t1_ranked = self._filter_and_score(
                official_pool,
                consumed_ids,
                ref_time,
                count,
                topics=norm_topics,
                from_date=from_dt,
                to_date=to_dt,
            )
            diagnostics["tier_1_valid_count"] = len(t1_ranked)
            if t1_ranked:
                self.store.save_cached_candidates(t1_ranked)
                return DiscoveryResult(
                    tier="tier_1_curated_official",
                    status="ELIGIBLE",
                    candidates=t1_ranked,
                    source_file="official_sources",
                    diagnostics=diagnostics,
                )
            return DiscoveryResult(
                tier="none",
                status="NO_ELIGIBLE_TOPIC",
                candidates=[],
                reason="No fresh unconsumed candidates found in official documentation sources matching criteria",
                diagnostics=diagnostics,
            )

        # --- Tier 2: Automated Live Feed (HackerNews, GitHub, ProductHunt, Reddit, ArXiv) ---
        # When querying daily news, live feeds take precedence over static documentation.
        diagnostics["tier_2_checked"] = True
        active_sources = [s for s in (feed_sources or DEFAULT_SOURCES) if s != "official"]
        if not active_sources:
            active_sources = list(DEFAULT_SOURCES)

        live_raw = self.fetcher.fetch_newsnow_feed(sources=active_sources)
        diagnostics["tier_2_raw_count"] = len(live_raw)

        if live_raw:
            t2_ranked = self._filter_and_score(
                live_raw,
                consumed_ids,
                ref_time,
                count,
                topics=norm_topics,
                from_date=from_dt,
                to_date=to_dt,
            )
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

        # --- Tier 1 Fallback: Official Sources (only if fresh and unconsumed) ---
        diagnostics["tier_1_fallback_checked"] = True
        official_pool = self.fetcher.fetch_official_sources()
        if official_pool:
            t1_fallback_ranked = self._filter_and_score(
                official_pool,
                consumed_ids,
                ref_time,
                count,
                topics=norm_topics,
                from_date=from_dt,
                to_date=to_dt,
            )
            diagnostics["tier_1_fallback_valid_count"] = len(t1_fallback_ranked)
            if t1_fallback_ranked:
                self.store.save_cached_candidates(t1_fallback_ranked)
                return DiscoveryResult(
                    tier="tier_1_curated_official_fallback",
                    status="ELIGIBLE",
                    candidates=t1_fallback_ranked,
                    source_file="official_sources",
                    diagnostics=diagnostics,
                )

        # --- Fallback: Cached candidates ---
        cached = self.store.get_cached_candidates()
        cached_unconsumed = [c for c in cached if c.get("candidate_id") not in consumed_ids]
        if cached_unconsumed:
            effective_cutoff = from_dt if from_dt else (ref_time - timedelta(days=self.max_age_days) if self.require_latest_news else None)
            bounded_cached = []
            for c in cached_unconsumed:
                ts = c.get("updated_at") or c.get("published_at") or c.get("published_at_utc")
                if ts:
                    try:
                        cand_dt = datetime.fromisoformat(str(ts).replace("Z", "+00:00"))
                        if cand_dt.tzinfo is None:
                            cand_dt = cand_dt.replace(tzinfo=timezone.utc)
                        if effective_cutoff and cand_dt < effective_cutoff:
                            continue
                        if to_dt and cand_dt > to_dt:
                            continue
                        bounded_cached.append(c)
                    except Exception:
                        pass
                elif not effective_cutoff and not to_dt:
                    bounded_cached.append(c)
            cached_unconsumed = bounded_cached

        if cached_unconsumed and norm_topics:
            target_set = {t.lower() for t in norm_topics}
            cached_unconsumed = [
                c for c in cached_unconsumed
                if (
                    str(c.get("topic_family") or "").lower() in target_set
                    or any(str(tag).lower() in target_set for tag in c.get("tags", []))
                    or any(tgt in str(c.get("topic_family") or "").lower() for tgt in target_set)
                    or any(tgt in str(c.get("title") or "").lower() for tgt in target_set)
                )
            ]

        if cached_unconsumed:
            return DiscoveryResult(
                tier="tier_2_cache",
                status="ELIGIBLE",
                candidates=cached_unconsumed[:count],
                source_file="cached_candidates",
                diagnostics=diagnostics,
            )

        # --- Tier 3: Clean Fail-Closed ---
        fail_reasons = []
        if norm_topics:
            fail_reasons.append(f"topic filter(s): {norm_topics}")
        if from_dt or to_dt:
            date_filter_desc = f"date range: [{from_dt.isoformat() if from_dt else '-inf'}, {to_dt.isoformat() if to_dt else '+inf'}]"
            fail_reasons.append(date_filter_desc)
        if fail_reasons:
            fail_reason = f"No eligible candidates matched the requested {', '.join(fail_reasons)}"
        else:
            fail_reason = "All candidates were consumed, stale, or dropped by safety/domain policies"

        return DiscoveryResult(
            tier="none",
            status="NO_ELIGIBLE_TOPIC",
            candidates=[],
            reason=fail_reason,
            diagnostics=diagnostics,
        )
