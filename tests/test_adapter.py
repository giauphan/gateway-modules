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


if __name__ == "__main__":
    unittest.main()
