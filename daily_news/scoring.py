"""Deterministic multi-axis scoring and ranking for news candidates.

Clean Architecture Layer: Domain Services / Application Business Rules.
Scores candidates along six explainable axes to rank content objectively.
"""

from __future__ import annotations

import datetime
from typing import Any, Dict, List, Optional

from .domain import NormalizedCandidate


class CandidateScorer:
    """Scores normalized candidates deterministically along six primary axes."""

    TIER_1_PUBLISHERS = {"reuters", "ap", "bloomberg", "wsj", "ft", "nytimes", "washingtonpost"}
    TIER_2_PUBLISHERS = {
        "theverge", "techcrunch", "wired", "arstechnica", "bbc", "npr", "cnbc", "guardian",
        "github", "hackernews", "ycombinator", "producthunt",
    }

    VISUAL_KEYWORDS = {
        "chart", "graph", "photo", "video", "announced", "design", "prototype",
        "revealed", "leaked", "interface", "hardware", "demo", "release",
    }

    INTEREST_KEYWORDS = {
        "ai", "artificial intelligence", "tech", "technology", "software", "hardware",
        "robot", "robotics", "machine learning", "deep learning", "llm", "model",
        "gpt", "claude", "gemini", "openai", "google", "microsoft", "apple", "nvidia",
        "chip", "semiconductor", "cyber", "security", "code", "coding", "developer",
        "app", "application", "data", "computing", "algorithm", "startup", "web",
        "digital", "cloud", "quantum", "browser", "linux", "open-source", "git", "api",
        "rust", "python", "agent", "agents",
    }

    def __init__(self, reference_time: Optional[datetime.datetime] = None) -> None:
        if reference_time is None:
            self.reference_time = datetime.datetime.now(datetime.timezone.utc)
        elif reference_time.tzinfo is None:
            self.reference_time = reference_time.replace(tzinfo=datetime.timezone.utc)
        else:
            self.reference_time = reference_time.astimezone(datetime.timezone.utc)

    def score_freshness(self, dt: datetime.datetime) -> float:
        """Decay curve: <=2h: 1.0; 2-12h: decay to 0.5; 12-24h: decay to 0.2; >24h: 0.1."""
        age_hours = max(0.0, (self.reference_time - dt).total_seconds() / 3600.0)
        if age_hours <= 2.0:
            return 1.0
        if age_hours <= 12.0:
            return 1.0 - 0.5 * ((age_hours - 2.0) / 10.0)
        if age_hours <= 24.0:
            return 0.5 - 0.3 * ((age_hours - 12.0) / 12.0)
        return 0.1

    def score_publisher(self, norm_pub: str) -> float:
        """Score publisher authority tier."""
        p = norm_pub.lower().strip()
        if p in self.TIER_1_PUBLISHERS:
            return 1.0
        if p in self.TIER_2_PUBLISHERS:
            return 0.8
        if p and p != "unknown":
            return 0.5
        return 0.2

    def score_fact_density(self, title: str, body: str) -> float:
        """Length and entity presence proxy."""
        text = f"{title} {body}".strip()
        if not text:
            return 0.1

        words = text.split()
        length = len(words)
        base = 1.0 if length > 400 else (0.8 if length > 200 else (0.5 if length > 50 else 0.3))

        has_quotes = '"' in text or "'" in text
        has_numbers = any(c.isdigit() for c in text)
        bonus = (0.1 if has_quotes else 0.0) + (0.1 if has_numbers else 0.0)
        return min(1.0, base + bonus)

    def score_visualizability(self, title: str, body: str, has_image: bool) -> float:
        """Score video or visualizable potential."""
        text = f"{title} {body}".lower()
        score = 0.3 + (0.3 if has_image else 0.0)
        matches = sum(1 for kw in self.VISUAL_KEYWORDS if kw in text)
        if matches >= 2:
            score += 0.4
        elif matches == 1:
            score += 0.2
        return min(1.0, score)

    def score_audience_interest(self, title: str, body: str) -> float:
        """Score tech relevance; penalize off-tech topics."""
        text = f"{title} {body}".lower()
        matches = sum(1 for kw in self.INTEREST_KEYWORDS if kw in text)
        if matches == 0:
            return 0.1
        if matches >= 2:
            return 1.0
        return 0.75

    def score_candidate(self, candidate: NormalizedCandidate) -> Dict[str, Any]:
        """Compute all axis scores and weighted overall total."""
        dt = datetime.datetime.fromisoformat(candidate.published_at_utc)
        body = candidate.body or candidate.description or ""
        has_image = bool(candidate.image_url)

        freshness = self.score_freshness(dt)
        publisher = self.score_publisher(candidate.normalized_publisher)
        fact_density = self.score_fact_density(candidate.title, body)
        visual = self.score_visualizability(candidate.title, body, has_image)
        interest = self.score_audience_interest(candidate.title, body)
        feasibility = 1.0

        total = (
            freshness * 0.30
            + publisher * 0.20
            + fact_density * 0.20
            + visual * 0.15
            + interest * 0.10
            + feasibility * 0.05
        )

        return {
            "total_score": round(total, 4),
            "axes": {
                "freshness": round(freshness, 4),
                "publisher_authority": round(publisher, 4),
                "fact_density": round(fact_density, 4),
                "visual_potential": round(visual, 4),
                "audience_interest": round(interest, 4),
                "feasibility": round(feasibility, 4),
            },
        }

    def rank_candidates(
        self,
        candidates: List[NormalizedCandidate],
        limit: int = 10,
    ) -> List[Dict[str, Any]]:
        """Score and sort candidates DESC by total score with stable tie-break on candidate_id."""
        scored: List[Dict[str, Any]] = []
        for c in candidates:
            d = c.to_dict()
            d["scoring"] = self.score_candidate(c)
            scored.append(d)

        scored.sort(key=lambda x: (-x["scoring"]["total_score"], x["candidate_id"]))
        return scored[:limit]
