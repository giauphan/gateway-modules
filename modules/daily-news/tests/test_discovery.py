"""Unit tests for TieredContentDiscovery and consumed tracking."""

from datetime import datetime, timedelta, timezone
import shutil
import tempfile
import unittest

from daily_news.discovery import TieredContentDiscovery
from daily_news.storage import NewsStateStore


class TestTieredDiscovery(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()
        self.store = NewsStateStore(self.temp_dir)
        self.discovery = TieredContentDiscovery(store=self.store)
        self.recent_ts = (datetime.now(timezone.utc) - timedelta(hours=2)).isoformat()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_tier_1_curated_priority(self):
        curated = [
            {
                "id": "c1",
                "url": "https://docs.anthropic.com/model-context-protocol",
                "title": "Model Context Protocol Specification 1.0",
                "publisher": "Anthropic",
                "published_at": self.recent_ts,
                "body": "Official specification release for agentic interfaces.",
            }
        ]
        res = self.discovery.discover_candidates(count=1, curated_feed=curated)
        self.assertTrue(res.is_eligible)
        self.assertEqual(res.tier, "tier_1_curated")
        self.assertEqual(len(res.candidates), 1)

    def test_consumed_filtering(self):
        curated = [
            {
                "id": "c1",
                "url": "https://github.com/torvalds/linux",
                "title": "Linux Kernel 7.0 Announced",
                "publisher": "GitHub",
                "published_at": self.recent_ts,
            }
        ]
        res1 = self.discovery.discover_candidates(count=1, curated_feed=curated)
        self.assertTrue(res1.is_eligible)
        cid = res1.candidates[0]["candidate_id"]

        # Mark consumed
        self.store.mark_consumed([cid])

        # Discovery again with same curated item should now fail-closed
        res2 = self.discovery.discover_candidates(count=1, curated_feed=curated)
        self.assertEqual(res2.status, "NO_ELIGIBLE_TOPIC")
        self.assertFalse(res2.is_eligible)

    def test_live_feed_priority_over_stale_official(self):
        class MockFetcher:
            def __init__(self, recent_ts):
                self.recent_ts = recent_ts

            def fetch_official_sources(self):
                return [
                    {
                        "id": "stale_1",
                        "url": "https://docs.anthropic.com/en/docs/build-with-claude/context-windows",
                        "title": "Old Docs From 2024",
                        "published_at": "2024-03-04T00:00:00Z",
                        "is_evergreen": True,
                    }
                ]

            def fetch_newsnow_feed(self, sources):
                return [
                    {
                        "id": "live_1",
                        "url": "https://news.ycombinator.com/item?id=99999",
                        "title": "Breaking Tech News Today",
                        "publisher": "news.ycombinator.com",
                        "published_at": self.recent_ts,
                    }
                ]

        mock_fetcher = MockFetcher(self.recent_ts)
        disc = TieredContentDiscovery(store=self.store, fetcher=mock_fetcher, require_latest_news=True)
        res = disc.discover_candidates(count=1)

        self.assertTrue(res.is_eligible)
        self.assertEqual(res.tier, "tier_2_automated")
        self.assertEqual(res.candidates[0]["title"], "Breaking Tech News Today")
        # Ensure stale 2024 doc was NOT returned
        self.assertNotIn("2024-03-04", [c.get("published_at") for c in res.candidates])

    def test_stale_official_sources_filtered_out_when_latest_required(self):
        class StaleOnlyFetcher:
            def fetch_official_sources(self):
                return [
                    {
                        "id": "stale_1",
                        "url": "https://docs.anthropic.com/en/docs/build-with-claude/context-windows",
                        "title": "Old Docs From 2024",
                        "published_at": "2024-03-04T00:00:00Z",
                        "is_evergreen": True,
                    }
                ]

            def fetch_newsnow_feed(self, sources):
                return []

        mock_fetcher = StaleOnlyFetcher()
        disc = TieredContentDiscovery(store=self.store, fetcher=mock_fetcher, require_latest_news=True)
        res = disc.discover_candidates(count=1)

        # Stale 2024 items must not be returned when require_latest_news is True
        self.assertFalse(res.is_eligible)
        self.assertEqual(res.status, "NO_ELIGIBLE_TOPIC")
        self.assertEqual(len(res.candidates), 0)

    def test_topic_filter_matching_and_fail_closed(self):
        class MultiTopicFetcher:
            def __init__(self, recent_ts):
                self.recent_ts = recent_ts

            def fetch_official_sources(self):
                return []

            def fetch_newsnow_feed(self, sources):
                return [
                    {
                        "id": "t1",
                        "url": "https://mcp.io/spec",
                        "title": "Model Context Protocol Tool Schema Update",
                        "published_at": self.recent_ts,
                        "description": "MCP function calls and tool schemas",
                    },
                    {
                        "id": "t2",
                        "url": "https://ci.dev/flaky",
                        "title": "Debugging Flaky Test Retries in CI",
                        "published_at": self.recent_ts,
                        "description": "Flaky CI loop debug and crash traces",
                    },
                ]

        mock_fetcher = MultiTopicFetcher(self.recent_ts)
        disc = TieredContentDiscovery(store=self.store, fetcher=mock_fetcher, require_latest_news=True)

        # 1. Filter by "agent_tool_use"
        res_tool = disc.discover_candidates(count=5, topics=["agent_tool_use"])
        self.assertTrue(res_tool.is_eligible)
        self.assertEqual(len(res_tool.candidates), 1)
        self.assertEqual(res_tool.candidates[0]["topic_family"], "agent_tool_use")
        self.assertEqual(res_tool.diagnostics["topics_filter"], ["agent_tool_use"])

        # 2. Filter by multiple topics
        res_multi = disc.discover_candidates(count=5, topics=["agent_tool_use", "debugging_reliability"])
        self.assertTrue(res_multi.is_eligible)
        self.assertEqual(len(res_multi.candidates), 2)

        # 3. Filter by non-existent topic fails closed
        res_none = disc.discover_candidates(count=5, topics=["crypto_blockchain_unknown"])
        self.assertFalse(res_none.is_eligible)
        self.assertEqual(res_none.status, "NO_ELIGIBLE_TOPIC")
        self.assertIn("crypto_blockchain_unknown", res_none.reason)


if __name__ == "__main__":
    unittest.main()
