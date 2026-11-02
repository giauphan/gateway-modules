"""Unit tests for NewsNormalizer and batch deduplication."""

from datetime import datetime, timedelta, timezone
import unittest

from daily_news.normalizer import NewsNormalizer


class TestNewsNormalizer(unittest.TestCase):
    def setUp(self):
        self.normalizer = NewsNormalizer(freshness_window_hours=48.0)
        self.now = datetime(2026, 11, 2, 12, 0, 0, tzinfo=timezone.utc)

    def test_normalize_valid_item(self):
        raw = {
            "id": "item_123",
            "url": "https://techcrunch.com/2026/11/02/ai-agents/?utm_source=feed",
            "title": "Anthropic Announces MCP Improvements",
            "publisher": "TechCrunch.com",
            "published_at": "2026-11-02T10:00:00Z",
            "description": "Anthropic published improvements to MCP.",
            "tags": ["ai", "mcp"],
        }
        cand = self.normalizer.normalize_item(raw)
        self.assertEqual(cand.candidate_id.startswith("cand_"), True)
        self.assertEqual(cand.canonical_url, "https://techcrunch.com/2026/11/02/ai-agents")
        self.assertEqual(cand.normalized_publisher, "TechCrunch")
        self.assertEqual(cand.tags, ["ai", "mcp"])

    def test_missing_required_fields_raises(self):
        with self.assertRaises(ValueError):
            self.normalizer.normalize_item({"id": "1", "url": "https://x.com", "title": ""})

    def test_batch_ingest_and_deduplicate(self):
        raw1 = {
            "id": "1",
            "url": "https://news.ycombinator.com/item?id=100",
            "title": "New Rust Toolcase Gateway Released",
            "published_at": "2026-11-02T11:00:00Z",
        }
        # Duplicate with same canonical content
        raw2 = {
            "id": "2",
            "url": "https://news.ycombinator.com/item?id=100&utm_source=twitter",
            "title": "New Rust Toolcase Gateway Released",
            "published_at": "2026-11-02T11:05:00Z",
        }
        # Stale item (> 48h)
        raw_stale = {
            "id": "3",
            "url": "https://news.ycombinator.com/item?id=50",
            "title": "Ancient News From Last Month",
            "published_at": "2026-10-25T11:00:00Z",
        }
        # Future item
        raw_future = {
            "id": "4",
            "url": "https://news.ycombinator.com/item?id=200",
            "title": "Time Traveler Post",
            "published_at": "2026-11-05T11:00:00Z",
        }

        candidates, diagnostics = self.normalizer.ingest_and_normalize(
            [raw1, raw2, raw_stale, raw_future],
            reference_time=self.now,
        )

        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].original_url, raw1["url"])
        self.assertEqual(len(diagnostics), 3)


if __name__ == "__main__":
    unittest.main()
