"""Unit tests for CandidateScorer and ranking axes."""

from datetime import datetime, timezone
import unittest

from daily_news.domain import NormalizedCandidate
from daily_news.scoring import CandidateScorer


class TestCandidateScorer(unittest.TestCase):
    def setUp(self):
        self.ref = datetime(2026, 11, 2, 12, 0, 0, tzinfo=timezone.utc)
        self.scorer = CandidateScorer(reference_time=self.ref)

    def _make_candidate(self, cid: str, title: str, pub: str, pub_time: str, body: str = ""):
        return NormalizedCandidate(
            candidate_id=cid,
            canonical_url=f"https://example.com/{cid}",
            original_url=f"https://example.com/{cid}",
            title=title,
            publisher=pub,
            normalized_publisher=pub,
            published_at_utc=pub_time,
            content_hash=f"hash_{cid}",
            title_hash=f"thash_{cid}",
            raw_provenance={"source_id": cid},
            body=body,
        )

    def test_freshness_decay(self):
        # 1 hour ago
        score_1h = self.scorer.score_freshness(datetime(2026, 11, 2, 11, 0, 0, tzinfo=timezone.utc))
        self.assertEqual(score_1h, 1.0)

        # 18 hours ago
        score_18h = self.scorer.score_freshness(datetime(2026, 11, 1, 18, 0, 0, tzinfo=timezone.utc))
        self.assertTrue(0.1 < score_18h < 0.5)

    def test_publisher_authority(self):
        self.assertEqual(self.scorer.score_publisher("reuters"), 1.0)
        self.assertEqual(self.scorer.score_publisher("theverge"), 0.8)
        self.assertEqual(self.scorer.score_publisher("unknown"), 0.2)

    def test_rank_candidates(self):
        c_high = self._make_candidate(
            "cand_1",
            "OpenAI Announces Breakthrough AI Model with Visual Demo",
            "reuters",
            "2026-11-02T11:00:00+00:00",
            body="New neural architecture with 50% lower latency and 'quoted' benchmark results.",
        )
        c_low = self._make_candidate(
            "cand_2",
            "Local Bakery Sells Muffins",
            "unknown",
            "2026-11-01T10:00:00+00:00",
            body="Fresh pastries every morning.",
        )

        ranked = self.scorer.rank_candidates([c_low, c_high], limit=10)
        self.assertEqual(len(ranked), 2)
        self.assertEqual(ranked[0]["candidate_id"], "cand_1")
        self.assertTrue(ranked[0]["scoring"]["total_score"] > ranked[1]["scoring"]["total_score"])


if __name__ == "__main__":
    unittest.main()
