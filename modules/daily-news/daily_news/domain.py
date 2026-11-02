"""Domain models and pure functions for daily news discovery and canonicalization.

Clean Architecture Layer: Domain Core (Entities and Enterprise Business Rules).
No dependencies on transport frameworks, HTTP handlers, or databases.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
import datetime
import hashlib
import ipaddress
import re
from typing import Any, Dict, List, Optional, Set
from urllib.parse import parse_qs, urlencode, urlparse, urlunparse

DEFAULT_ALLOWED_DOMAINS: Set[str] = {
    "docs.anthropic.com",
    "docs.github.com",
    "modelcontextprotocol.io",
    "microsoft.github.io",
    "openai.com",
    "arxiv.org",
    "github.com",
    "git-scm.com",
    "news.ycombinator.com",
    "producthunt.com",
    "techcrunch.com",
    "theverge.com",
    "wired.com",
    "arstechnica.com",
    "venturebeat.com",
}

APPROVED_TOPIC_FAMILIES: List[str] = [
    "context_memory",
    "agent_tool_use",
    "debugging_reliability",
    "agent_evaluation",
    "agent_architecture",
]

TRACKING_QUERY_PARAMS: Set[str] = {
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "fbclid", "gclid", "msclkid", "ref", "source", "rss",
}


@dataclass(frozen=True)
class RawNewsItem:
    """Incoming un-normalized news article representation."""

    id: str
    url: str
    title: str
    published_at: str
    publisher: Optional[str] = None
    author: Optional[str] = None
    description: Optional[str] = None
    body: Optional[str] = None
    content_hash: Optional[str] = None
    tags: List[str] = field(default_factory=list)
    language: Optional[str] = "en"
    image_url: Optional[str] = None
    source_feed: Optional[str] = None
    extra: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class NormalizedCandidate:
    """Mechanically normalized, deduplicated news candidate."""

    candidate_id: str
    canonical_url: str
    original_url: str
    title: str
    publisher: str
    normalized_publisher: str
    published_at_utc: str
    content_hash: str
    title_hash: str
    raw_provenance: Dict[str, Any]
    adapter_version: str = "1.0.0"
    topic_family: str = "agent_architecture"
    author: Optional[str] = None
    description: Optional[str] = None
    body: Optional[str] = None
    tags: List[str] = field(default_factory=list)
    language: Optional[str] = "en"
    image_url: Optional[str] = None
    is_evergreen: bool = False
    content_kind: str = "news"
    summary: Optional[str] = None
    hook: Optional[str] = None
    viewer_question: Optional[str] = None
    symptom: Optional[str] = None
    root_cause: Optional[str] = None
    remedy: Optional[str] = None
    evidence_excerpt: Optional[str] = None
    source_snapshot: Optional[str] = None
    snapshot_sha256: Optional[str] = None
    independent_fact_check: Optional[Dict[str, Any]] = None
    scoring: Optional[Dict[str, Any]] = None

    def to_dict(self) -> Dict[str, Any]:
        d = asdict(self)
        d["url"] = self.canonical_url
        d["published_at"] = self.published_at_utc
        if not d.get("summary"):
            d["summary"] = self.description or (self.body[:300] if self.body else "")
        return d


@dataclass(frozen=True)
class DiscoveryResult:
    """Outcome of tiered candidate discovery."""

    tier: str
    status: str
    candidates: List[Dict[str, Any]] = field(default_factory=list)
    source_file: Optional[str] = None
    reason: Optional[str] = None
    diagnostics: Dict[str, Any] = field(default_factory=dict)

    @property
    def is_eligible(self) -> bool:
        return self.status == "ELIGIBLE" and len(self.candidates) > 0

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


def classify_topic_family(title: str, text: str = "") -> str:
    """Classify candidate into one of the 5 approved topic families."""
    combined = f"{title} {text}".lower()
    if any(k in combined for k in ("prompt caching", "context window", "kv cache", "memory", "state loss", "token limit")):
        return "context_memory"
    if any(k in combined for k in ("tool call", "mcp", "schema", "function call", "tools", "payload parsing")):
        return "agent_tool_use"
    if any(k in combined for k in ("debug", "retry", "flaky", "reliability", "ci", "loop", "crash", "failure", "trace")):
        return "debugging_reliability"
    if any(k in combined for k in ("benchmark", "eval", "evaluation", "score", "metrics", "test suite")):
        return "agent_evaluation"
    return "agent_architecture"


def canonicalize_url(url: str) -> str:
    """Strip tracking query parameters, normalize scheme/host, and strip trailing slashes."""
    if not url:
        return ""
    parsed = urlparse(url.strip())
    scheme = parsed.scheme.lower()
    netloc = parsed.netloc.lower()
    path = parsed.path.rstrip("/")

    query_dict = parse_qs(parsed.query, keep_blank_values=False)
    filtered = {k: v for k, v in query_dict.items() if k.lower() not in TRACKING_QUERY_PARAMS}
    new_query = urlencode(filtered, doseq=True)

    return urlunparse((scheme, netloc, path, parsed.params, new_query, ""))


def normalize_publisher(publisher: Optional[str]) -> str:
    """Clean and standardize publisher strings, removing common suffixes."""
    if not publisher:
        return "Unknown"
    cleaned = re.sub(r"\s+", " ", publisher.strip())
    cleaned = re.sub(r"\.(com|org|net|co|io|gov|news)$", "", cleaned, flags=re.IGNORECASE)
    return cleaned.strip() or "Unknown"


def parse_iso_utc(ts_str: str) -> str:
    """Parse raw timestamp string and return canonical ISO-8601 UTC string."""
    ts = ts_str.strip()
    if ts.endswith("Z"):
        ts = ts[:-1] + "+00:00"

    dt: Optional[datetime.datetime] = None
    try:
        dt = datetime.datetime.fromisoformat(ts)
    except ValueError:
        for fmt in (
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d",
            "%a, %d %b %Y %H:%M:%S %z",
            "%a, %d %b %Y %H:%M:%S GMT",
        ):
            try:
                dt = datetime.datetime.strptime(ts, fmt)
                break
            except ValueError:
                pass

    if dt is None:
        raise ValueError(f"Unparseable timestamp: {ts_str}")

    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=datetime.timezone.utc)
    else:
        dt = dt.astimezone(datetime.timezone.utc)

    return dt.isoformat()


def compute_title_hash(title: str) -> str:
    """Compute SHA-256 digest of normalized headline."""
    norm = re.sub(r"\s+", " ", title.strip().lower())
    return hashlib.sha256(norm.encode("utf-8")).hexdigest()


def compute_content_hash(title: str, body: Optional[str], canonical_url: str) -> str:
    """Compute authoritative SHA-256 digest of content signature."""
    clean_title = re.sub(r"\s+", " ", title.strip().lower())
    clean_url = canonical_url.strip().lower()
    clean_body = re.sub(r"\s+", " ", (body or "").strip().lower())
    payload = f"{clean_title}|{clean_url}|{clean_body[:500]}"
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def is_safe_url(url: str, allowed_domains: Optional[Set[str]] = None) -> bool:
    """SSRF guard: validates scheme, host, and blocks private/loopback/link-local IPs."""
    if not url:
        return False
    try:
        parsed = urlparse(url)
        if parsed.scheme.lower() not in ("http", "https"):
            return False
        host = (parsed.hostname or "").lower()
        if not host:
            return False

        try:
            ip = ipaddress.ip_address(host)
            if ip.is_private or ip.is_loopback or ip.is_reserved or ip.is_link_local:
                if host in ("127.0.0.1", "::1", "localhost"):
                    return True
                return False
        except ValueError:
            pass

        if allowed_domains:
            return any(host == d or host.endswith("." + d) for d in allowed_domains)
        return True
    except Exception:
        return False
