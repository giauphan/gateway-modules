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

    def test_normalize_discussion_url(self):
        raw = {
            "id": "hn_50333339",
            "url": "https://wikihack.netlify.app",
            "title": "Show HN: Wikihack",
            "discussion_url": "https://news.ycombinator.com/item?id=50333339",
            "published_at": "2026-11-02T10:00:00Z",
        }
        cand = self.normalizer.normalize_item(raw)
        self.assertEqual(cand.canonical_url, "https://wikihack.netlify.app")
        self.assertEqual(cand.discussion_url, "https://news.ycombinator.com/item?id=50333339")
        d = cand.to_dict()
        self.assertEqual(d["canonical_url"], "https://wikihack.netlify.app")
        self.assertEqual(d["discussion_url"], "https://news.ycombinator.com/item?id=50333339")

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

    def test_evergreen_items_bypass_freshness_window(self):
        evergreen_raw = {
            "id": "eg_1",
            "url": "https://git-scm.com/docs/git-worktree",
            "title": "Git Worktree Documentation",
            "published_at": "2015-07-28T00:00:00Z",
            "updated_at": "2026-10-28T14:00:00Z",
            "is_evergreen": True,
            "content_kind": "documentation",
        }
        candidates, diagnostics = self.normalizer.ingest_and_normalize(
            [evergreen_raw],
            reference_time=self.now,
        )
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].is_evergreen, True)
        self.assertEqual(candidates[0].updated_at, "2026-10-28T14:00:00+00:00")
        self.assertEqual(candidates[0].published_at_utc, "2015-07-28T00:00:00+00:00")
        self.assertEqual(candidates[0].to_dict().get("updated_at"), "2026-10-28T14:00:00+00:00")
        self.assertEqual(len(diagnostics), 0)


    def test_require_latest_news_enforces_freshness_even_for_evergreen(self):
        # When require_latest_news=True, stale items (even evergreen from 2024 or 2015) MUST be dropped
        stale_2024_doc = {
            "id": "cand_official_anthropic_context_window",
            "url": "https://docs.anthropic.com/en/docs/build-with-claude/context-windows",
            "title": "Why coding agents lose project requirements between sessions",
            "published_at": "2024-03-04T00:00:00Z",
            "is_evergreen": True,
            "content_kind": "documentation",
        }
        candidates, diagnostics = self.normalizer.ingest_and_normalize(
            [stale_2024_doc],
            reference_time=self.now,
            require_latest_news=True,
        )
        self.assertEqual(len(candidates), 0)
        self.assertEqual(len(diagnostics), 1)
        self.assertEqual(diagnostics[0]["reason"], "stale_content")
        self.assertEqual(diagnostics[0]["effective_date"], "2024-03-04T00:00:00+00:00")

    def test_require_latest_news_accepts_recently_updated_items(self):
        # An item originally published in 2024 but updated within 48h must be accepted
        freshly_updated_doc = {
            "id": "doc_updated_today",
            "url": "https://docs.anthropic.com/en/docs/build-with-claude/context-windows",
            "title": "Context Windows Updates",
            "published_at": "2024-03-04T00:00:00Z",
            "updated_at": "2026-11-02T10:00:00Z",  # 2h before reference self.now
            "is_evergreen": True,
        }
        candidates, diagnostics = self.normalizer.ingest_and_normalize(
            [freshly_updated_doc],
            reference_time=self.now,
            require_latest_news=True,
        )
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].candidate_id.startswith("cand_"), True)
        self.assertEqual(candidates[0].updated_at, "2026-11-02T10:00:00+00:00")
        self.assertEqual(candidates[0].published_at_utc, "2024-03-04T00:00:00+00:00")
        self.assertEqual(len(diagnostics), 0)

    def test_from_and_to_date_filtering(self):
        # 3 items with distinct timestamps
        item1 = {
            "id": "i1",
            "url": "https://example.com/item1",
            "title": "Item on Oct 25",
            "published_at": "2026-10-25T12:00:00Z",
        }
        item2 = {
            "id": "i2",
            "url": "https://example.com/item2",
            "title": "Item on Oct 30",
            "published_at": "2026-10-30T12:00:00Z",
        }
        item3 = {
            "id": "i3",
            "url": "https://example.com/item3",
            "title": "Item on Nov 02",
            "published_at": "2026-11-02T10:00:00Z",
        }

        # Filter window: Oct 28 to Nov 01
        from_dt = datetime(2026, 10, 28, 0, 0, 0, tzinfo=timezone.utc)
        to_dt = datetime(2026, 11, 1, 23, 59, 59, tzinfo=timezone.utc)

        candidates, diagnostics = self.normalizer.ingest_and_normalize(
            [item1, item2, item3],
            reference_time=self.now,
            from_date=from_dt,
            to_date=to_dt,
        )

        # Only item2 (Oct 30) matches
        self.assertEqual(len(candidates), 1)
        self.assertEqual(candidates[0].original_url, item2["url"])

        # item1 dropped for stale/before from_date, item3 dropped for future/after to_date
        drop_reasons = {d["raw_item_id"]: d["reason"] for d in diagnostics}
        self.assertEqual(drop_reasons["i1"], "before_from_date")
        self.assertEqual(drop_reasons["i3"], "after_to_date")


if __name__ == "__main__":
    unittest.main()
