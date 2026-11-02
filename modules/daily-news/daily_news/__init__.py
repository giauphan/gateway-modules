"""Daily News & Content Discovery Package.

Clean Architecture facade exposing primary domain entities, application ports, and services.
"""

from .domain import (
    APPROVED_TOPIC_FAMILIES,
    DiscoveryResult,
    NormalizedCandidate,
    RawNewsItem,
    canonicalize_url,
    classify_topic_family,
    compute_content_hash,
    compute_title_hash,
    is_safe_url,
    normalize_publisher,
    parse_boundary_datetime,
    parse_iso_utc,
)
from .fetcher import DEFAULT_SOURCES, FeedFetcher
from .normalizer import NewsNormalizer
from .ports import (
    CandidateScorerPort,
    FeedFetcherPort,
    NewsNormalizerPort,
    NewsStateStorePort,
)
from .providers import RedisFeedProvider
from .scoring import CandidateScorer
from .discovery import TieredContentDiscovery
from .storage import NewsStateStore
from .ui import render_dashboard_html

__all__ = [
    "APPROVED_TOPIC_FAMILIES",
    "DiscoveryResult",
    "NormalizedCandidate",
    "RawNewsItem",
    "canonicalize_url",
    "classify_topic_family",
    "compute_content_hash",
    "compute_title_hash",
    "is_safe_url",
    "normalize_publisher",
    "parse_boundary_datetime",
    "parse_iso_utc",
    "DEFAULT_SOURCES",
    "FeedFetcher",
    "FeedFetcherPort",
    "RedisFeedProvider",
    "NewsNormalizer",
    "NewsNormalizerPort",
    "CandidateScorer",
    "CandidateScorerPort",
    "TieredContentDiscovery",
    "NewsStateStore",
    "NewsStateStorePort",
    "render_dashboard_html",
]
