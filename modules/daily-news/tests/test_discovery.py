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


if __name__ == "__main__":
    unittest.main()
