"""Unit tests for the stdio-stream protocol adapter."""

from datetime import datetime, timedelta, timezone
import json
import unittest

from adapter import handle_request


class TestAdapter(unittest.TestCase):
    def setUp(self):
        self.recent_ts = (datetime.now(timezone.utc) - timedelta(hours=2)).isoformat()

    def test_health_endpoint(self):
        req = {"path": "/health", "method": "GET"}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "healthy")
        self.assertEqual(body["module"], "daily-news")

    def test_models_endpoint(self):
        req = {"path": "/models", "method": "GET"}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["object"], "list")
        self.assertEqual(len(body["data"]), 1)
        self.assertEqual(body["data"][0]["id"], "daily-news")

    def test_ui_endpoint(self):
        req = {"path": "/ui", "method": "GET"}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        self.assertIn("text/html", resp["headers"]["content-type"])
        self.assertIn("Daily News &amp; Content Discovery", resp["body"])

    def test_candidates_endpoint_with_custom_items(self):
        raw_items = [
            {
                "id": "item-1",
                "url": "https://docs.anthropic.com/agent-tools",
                "title": "Anthropic Enhances Tool Use in Claude",
                "publisher": "Anthropic",
                "published_at": self.recent_ts,
                "body": "Detailed guide on tool invocation with schema enforcement.",
            }
        ]
        req = {
            "path": "/api/v1/candidates",
            "method": "POST",
            "body": json.dumps({"items": raw_items, "limit": 5}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["count"], 1)
        self.assertEqual(body["candidates"][0]["title"], "Anthropic Enhances Tool Use in Claude")

    def test_consume_endpoint(self):
        req = {
            "path": "/api/v1/consume",
            "method": "POST",
            "body": json.dumps({"candidate_ids": ["cand_test_123"]}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ok")

    def test_discover_endpoint_topic_filtering(self):
        curated = [
            {
                "id": "c_tool",
                "url": "https://mcp.io/tool-spec",
                "title": "Model Context Protocol Tool Schema Spec",
                "publisher": "MCP",
                "published_at": self.recent_ts,
                "body": "Tool call schemas and function parameters.",
            },
            {
                "id": "c_debug",
                "url": "https://debug.io/trace",
                "title": "Debugging Flaky Test Retries in CI",
                "publisher": "DebugIO",
                "published_at": self.recent_ts,
                "body": "Crash traces and retry loops.",
            },
        ]

        # 1. Singular topic filter
        req_single = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({
                "curated_candidates": curated,
                "topic": "agent_tool_use",
            }),
        }
        resp = handle_request(req_single)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ELIGIBLE")
        self.assertEqual(len(body["candidates"]), 1)
        self.assertEqual(body["candidates"][0]["topic_family"], "agent_tool_use")

        # 2. Plural topics filter
        req_multi = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({
                "curated_candidates": curated,
                "topics": ["agent_tool_use", "debugging_reliability"],
            }),
        }
        resp = handle_request(req_multi)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ELIGIBLE")
        self.assertEqual(len(body["candidates"]), 2)

    def test_discover_limit_and_count_aliases(self):
        items = [
            {
                "id": f"c_{i}",
                "url": f"https://example.com/art-{i}",
                "title": f"Article {i}",
                "publisher": "Example",
                "published_at": self.recent_ts,
            }
            for i in range(10)
        ]
        # Request with "limit"
        req1 = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({"curated_candidates": items, "limit": 4}),
        }
        res1 = handle_request(req1)
        self.assertEqual(res1["status"], 200)
        self.assertEqual(len(json.loads(res1["body"])["candidates"]), 4)

        # Request with "count"
        req2 = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({"curated_candidates": items, "count": 6}),
        }
        res2 = handle_request(req2)
        self.assertEqual(res2["status"], 200)
        self.assertEqual(len(json.loads(res2["body"])["candidates"]), 6)

    def test_discover_query_string_parameters(self):
        items = [
            {
                "id": f"c_{i}",
                "url": f"https://example.com/art-{i}",
                "title": f"Article {i}",
                "publisher": "Example",
                "published_at": self.recent_ts,
            }
            for i in range(10)
        ]
        # GET request with ?limit=3 in query string
        req1 = {
            "path": "/api/v1/discover?limit=3",
            "method": "GET",
            "body": json.dumps({"curated_candidates": items}),
        }
        res1 = handle_request(req1)
        self.assertEqual(res1["status"], 200)
        self.assertEqual(len(json.loads(res1["body"])["candidates"]), 3)

        # GET request with ?count=7 in query string
        req2 = {
            "path": "/api/v1/discover?count=7",
            "method": "GET",
            "body": json.dumps({"curated_candidates": items}),
        }
        res2 = handle_request(req2)
        self.assertEqual(res2["status"], 200)
        self.assertEqual(len(json.loads(res2["body"])["candidates"]), 7)

    def test_discover_endpoint_date_range_filtering(self):
        curated = [
            {
                "id": "c_oct29",
                "url": "https://mcp.io/tool-spec-1",
                "title": "Model Context Protocol Tool Schema Spec",
                "publisher": "MCP",
                "published_at": "2026-10-29T10:00:00Z",
            },
            {
                "id": "c_oct31",
                "url": "https://debug.io/trace-2",
                "title": "Debugging Flaky Test Retries in CI",
                "publisher": "DebugIO",
                "published_at": "2026-10-31T15:00:00Z",
            },
            {
                "id": "c_nov02",
                "url": "https://news.ycombinator.com/item?id=300",
                "title": "New Toolcase Release",
                "publisher": "HackerNews",
                "published_at": self.recent_ts,
            },
        ]

        # 1. Search date range Oct 30 to Nov 01
        req = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({
                "curated_candidates": curated,
                "from_date": "2026-10-30",
                "to_date": "2026-11-01",
            }),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ELIGIBLE")
        self.assertEqual(len(body["candidates"]), 1)
        self.assertEqual(body["candidates"][0]["title"], "Debugging Flaky Test Retries in CI")

        # 2. Inverted date range returns 400 Bad Request
        req_inverted = {
            "path": "/api/v1/discover",
            "method": "POST",
            "body": json.dumps({
                "curated_candidates": curated,
                "from_date": "2026-11-05",
                "to_date": "2026-11-01",
            }),
        }
        resp_inverted = handle_request(req_inverted)
        self.assertEqual(resp_inverted["status"], 400)
        body_inv = json.loads(resp_inverted["body"])
        self.assertEqual(body_inv["status"], "INVALID_DATE_RANGE")

    def test_adapter_redis_config_and_feed(self):
        # 1. Update config with Redis settings
        req_cfg = {
            "path": "/api/v1/config",
            "method": "POST",
            "body": json.dumps({
                "redis_enabled": True,
                "redis_url": "redis://127.0.0.1:6379/1",
                "redis_key": "custom:news:stream",
                "redis_type": "stream",
            }),
        }
        resp_cfg = handle_request(req_cfg)
        self.assertEqual(resp_cfg["status"], 200)
        cfg = json.loads(resp_cfg["body"])["config"]
        self.assertEqual(cfg["redis_enabled"], True)
        self.assertEqual(cfg["redis_key"], "custom:news:stream")

        # 2. Query /api/v1/feed with sources=["redis"]
        # Redis is offline locally, so it must return 0 items gracefully without raising 500
        req_feed = {
            "path": "/api/v1/feed",
            "method": "POST",
            "body": json.dumps({"sources": ["redis"]}),
        }
        resp_feed = handle_request(req_feed)
        self.assertEqual(resp_feed["status"], 200)
        feed_body = json.loads(resp_feed["body"])
        self.assertEqual(isinstance(feed_body["items"], list), True)

    def test_chat_completions_with_discussion_url(self):
        req = {
            "path": "/chat/completions",
            "method": "POST",
            "body": json.dumps({"limit": 2}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        content = body["choices"][0]["message"]["content"]
        self.assertIn("Daily News Briefing", content)


if __name__ == "__main__":
    unittest.main()
