"""Unit tests for Redis Queue/Stream Feed Provider and FeedFetcher integration."""

from datetime import datetime, timezone
import json
import unittest
from unittest.mock import MagicMock

from daily_news.fetcher import FeedFetcher
from daily_news.providers.redis_provider import RedisFeedProvider


class TestRedisFeedProvider(unittest.TestCase):
    def test_redis_stream_xrevrange_ingestion(self):
        mock_client = MagicMock()
        mock_client.xrevrange.return_value = [
            (
                "1730541600000-0",
                {
                    "title": "Anthropic Releases MCP Server Dynamic Tool Filter",
                    "url": "https://modelcontextprotocol.io/tools/filtering",
                    "publisher": "MCP Standard",
                    "published_at": "2026-11-02T10:00:00Z",
                    "body": "Detailed guide on dynamic MCP tool schema filtering.",
                    "topic_family": "agent_tool_use",
                    "symptom": "Agent context window bloat",
                    "root_cause": "Loading 50+ tool schemas simultaneously",
                    "remedy": "Two-tier tool resolution",
                },
            ),
            (
                "1730541500000-0",
                {
                    # Payload as embedded JSON string
                    "payload": json.dumps({
                        "title": "Debugging Flaky Test Retries in Agent Loops",
                        "url": "https://example.com/agent-loops",
                        "publisher": "Agent CI",
                        "published_at": "2026-11-02T09:45:00Z",
                        "body": "Diagnosing infinite retry loops in autonomous coding agents.",
                        "topic_family": "debugging_reliability",
                    })
                },
            ),
        ]

        provider = RedisFeedProvider(
            redis_url="redis://localhost:6379/0",
            key="news:incoming",
            channel_type="stream",
            client=mock_client,
        )

        items = provider.fetch_items(limit=5)
        self.assertEqual(len(items), 2)
        mock_client.xrevrange.assert_called_once_with("news:incoming", max="+", min="-", count=5)

        # Check item 1 (discrete hash fields)
        item1 = items[0]
        self.assertEqual(item1["title"], "Anthropic Releases MCP Server Dynamic Tool Filter")
        self.assertEqual(item1["url"], "https://modelcontextprotocol.io/tools/filtering")
        self.assertEqual(item1["publisher"], "MCP Standard")
        self.assertEqual(item1["topic_family"], "agent_tool_use")
        self.assertEqual(item1["symptom"], "Agent context window bloat")
        self.assertEqual(item1["source_feed"], "redis:news:incoming")
        self.assertEqual(item1["extra"]["msg_id"], "1730541600000-0")

        # Check item 2 (embedded JSON payload string)
        item2 = items[1]
        self.assertEqual(item2["title"], "Debugging Flaky Test Retries in Agent Loops")
        self.assertEqual(item2["url"], "https://example.com/agent-loops")
        self.assertEqual(item2["topic_family"], "debugging_reliability")

    def test_redis_list_lrange_ingestion(self):
        mock_client = MagicMock()
        mock_client.lrange.return_value = [
            json.dumps({
                "title": "Autonomous Coding Agent Benchmark Matrix",
                "url": "https://example.com/benchmark",
                "publisher": "AgentBench",
                "published_at": "2026-11-02T08:00:00Z",
                "body": "Comprehensive evaluation matrix across 200 coding agent runs.",
                "topic_family": "agent_evaluation",
            })
        ]

        provider = RedisFeedProvider(
            redis_url="redis://localhost:6379/0",
            key="news:queue",
            channel_type="list",
            client=mock_client,
        )

        items = provider.fetch_items(limit=10)
        self.assertEqual(len(items), 1)
        mock_client.lrange.assert_called_once_with("news:queue", 0, 9)
        self.assertEqual(items[0]["title"], "Autonomous Coding Agent Benchmark Matrix")
        self.assertEqual(items[0]["topic_family"], "agent_evaluation")

    def test_timestamp_from_stream_msg_id_fallback(self):
        mock_client = MagicMock()
        # Message with no published_at field; ID is timestamp 1730541600000 (2024-11-02 10:00:00 UTC)
        mock_client.xrevrange.return_value = [
            (
                "1730541600000-0",
                {
                    "title": "Stream Item With Auto Timestamp",
                    "url": "https://example.com/auto-ts",
                },
            )
        ]

        provider = RedisFeedProvider(channel_type="stream", client=mock_client)
        items = provider.fetch_items(limit=1)
        self.assertEqual(len(items), 1)
        self.assertTrue("2024-11-02" in items[0]["published_at"] or "2026" in items[0]["published_at"])

    def test_connection_error_resilience(self):
        mock_client = MagicMock()
        mock_client.xrevrange.side_effect = ConnectionError("Connection refused by Redis")

        provider = RedisFeedProvider(
            redis_url="redis://nonexistent:6379/0",
            key="news:stream",
            channel_type="stream",
            client=mock_client,
        )

        # Must return empty list gracefully without raising
        items = provider.fetch_items(limit=10)
        self.assertEqual(items, [])

    def test_feed_fetcher_integration_with_redis(self):
        mock_redis = MagicMock()
        mock_redis.fetch_items.return_value = [
            {
                "id": "redis_1",
                "title": "Redis Ingested Article",
                "url": "https://example.com/redis-article",
                "publisher": "Redis Queue",
                "published_at": datetime.now(timezone.utc).isoformat(),
                "body": "Article body from redis queue",
                "tags": ["redis"],
                "source_feed": "redis:news:stream",
            }
        ]

        fetcher = FeedFetcher(redis_provider=mock_redis)
        items = fetcher.fetch_newsnow_feed(sources=["redis"])
        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["title"], "Redis Ingested Article")
        mock_redis.fetch_items.assert_called_once_with(limit=15)


if __name__ == "__main__":
    unittest.main()
