"""Daily News & Content Discovery Package.

Clean Architecture facade exposing primary domain entities and application services.
"""

from .domain import (
    DiscoveryResult,
    NormalizedCandidate,
    RawNewsItem,
    canonicalize_url,
    compute_content_hash,
    compute_title_hash,
    is_safe_url,
    normalize_publisher,
    parse_iso_utc,
)
from .fetcher import FeedFetcher
from .normalizer import NewsNormalizer
from .scoring import CandidateScorer
from .discovery import TieredContentDiscovery
from .storage import NewsStateStore
from .ui import render_dashboard_html

__all__ = [
    "DiscoveryResult",
    "NormalizedCandidate",
    "RawNewsItem",
    "canonicalize_url",
    "compute_content_hash",
    "compute_title_hash",
    "is_safe_url",
    "normalize_publisher",
    "parse_iso_utc",
    "FeedFetcher",
    "NewsNormalizer",
    "CandidateScorer",
    "TieredContentDiscovery",
    "NewsStateStore",
    "render_dashboard_html",
]
