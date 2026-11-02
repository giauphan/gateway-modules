"""Inward and Outward Port Protocols for the Daily News service.

Clean Architecture Layer: Application Ports (typing.Protocol).
Defines abstract contracts for secondary adapters (persistence, network, scoring)
enforcing the Dependency Inversion Principle (DIP).
"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Dict, List, Optional, Protocol, Sequence, Set, Tuple

from .domain import NormalizedCandidate


class FeedFetcherPort(Protocol):
    """Outbound port for fetching upstream technical and live feeds."""

    def fetch_official_sources(self) -> List[Dict[str, Any]]:
        ...

    def fetch_newsnow_feed(
        self,
        sources: Sequence[str],
    ) -> List[Dict[str, Any]]:
        ...


class NewsStateStorePort(Protocol):
    """Outbound port for state persistence and consumed candidate tracking."""

    def get_consumed_ids(self) -> Set[str]:
        ...

    def mark_consumed(self, candidate_ids: List[str] | Set[str]) -> None:
        ...

    def save_cached_candidates(self, candidates: List[Dict[str, Any]]) -> None:
        ...

    def get_cached_candidates(self) -> List[Dict[str, Any]]:
        ...


class CandidateScorerPort(Protocol):
    """Outbound port for deterministic candidate scoring and ranking."""

    def rank_candidates(
        self,
        candidates: List[NormalizedCandidate],
        limit: int = 10,
    ) -> List[Dict[str, Any]]:
        ...


class NewsNormalizerPort(Protocol):
    """Outbound port for candidate ingestion, canonicalization, and deduplication."""

    def ingest_and_normalize(
        self,
        raw_items: List[Dict[str, Any]],
        reference_time: Optional[datetime] = None,
        require_latest_news: Optional[bool] = None,
        from_date: Optional[datetime] = None,
        to_date: Optional[datetime] = None,
    ) -> Tuple[List[NormalizedCandidate], List[Dict[str, Any]]]:
        ...
