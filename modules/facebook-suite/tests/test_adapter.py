"""
Unit tests for Meta Business Suite stdio adapter routing.
"""

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import facebook_suite.client as client
from adapter import handle_request


class TestFacebookAdapter(unittest.TestCase):

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        p = mock.patch.object(client, "get_config_path", return_value=Path(self._tmp.name) / "config.json")
        p.start()
        self.addCleanup(p.stop)
        self.addCleanup(self._tmp.cleanup)
        client._RUNTIME_CONFIG_CACHE = None
        self.addCleanup(setattr, client, "_RUNTIME_CONFIG_CACHE", None)

    def test_ui_endpoint(self):
        req = {"method": "GET", "path": "/ui", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        self.assertIn("text/html", resp["headers"]["content-type"])
        self.assertIn("Meta Business Suite", resp["body"])

    def test_status_endpoint(self):
        req = {"method": "GET", "path": "/status", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["module"], "facebook-suite")
        self.assertIn("status", body)
        self.assertIn("configured", body)

    def test_health_endpoint(self):
        req = {"method": "GET", "path": "/health", "headers": {}, "body": "", "probe": True}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "healthy")
        self.assertTrue(body["ok"])
        self.assertTrue(body["healthy"])
        self.assertEqual(body["module"], "facebook-suite")

    def test_health_endpoint_with_namespace(self):
        req = {"method": "GET", "path": "/custom/facebook/health", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "healthy")
        self.assertTrue(body["ok"])

    def test_health_when_disabled(self):
        client.save_module_config({"enabled": False})
        req = {"method": "GET", "path": "/health", "headers": {}, "body": "", "probe": True}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "healthy")
        self.assertTrue(body["ok"])

    def test_config_endpoints(self):
        # GET config
        req_get = {"method": "GET", "path": "/config", "headers": {}, "body": ""}
        resp_get = handle_request(req_get)
        self.assertEqual(resp_get["status"], 200)

        # POST config
        new_cfg = {"target_id": "987654321", "actor_id": "987654321", "user_id": "123456789", "enabled": True}
        req_post = {
            "method": "POST",
            "path": "/config",
            "headers": {"content-type": "application/json"},
            "body": json.dumps(new_cfg),
        }
        resp_post = handle_request(req_post)
        self.assertEqual(resp_post["status"], 200)
        body = json.loads(resp_post["body"])
        self.assertEqual(body["status"], "ok")

    def test_har_import_endpoint(self):
        synthetic_har = {
            "log": {
                "entries": [
                    {
                        "request": {
                            "url": "https://business.facebook.com/api/graphql/?av=987654321",
                            "method": "POST",
                            "headers": [
                                {"name": "Cookie", "value": "c_user=123456789; xs=testxs;"},
                                {"name": "User-Agent", "value": "TestAgent/1.0"},
                            ],
                            "postData": {
                                "text": "target_id=987654321&__user=123456789&fb_dtsg=test_dtsg_token:1:12345"
                            },
                        }
                    }
                ]
            }
        }
        req = {
            "method": "POST",
            "path": "/v1/config/har",
            "headers": {"content-type": "application/json"},
            "body": json.dumps(synthetic_har),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ok")
        self.assertTrue(body["configured"])

    @mock.patch("urllib.request.urlopen")
    def test_scopes_endpoint(self, _mock_urlopen):
        client.save_module_config({"user_id": "123456789", "actor_id": "987654321", "enabled": True})
        req = {"method": "GET", "path": "/v1/scopes", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["status"], "ok")
        self.assertIn("scopes", body)

    def test_models_facade(self):
        req = {"method": "GET", "path": "/models", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["object"], "list")
        self.assertEqual(body["data"][0]["id"], "facebook-suite")

    def test_chat_completions_facade(self):
        req = {
            "method": "POST",
            "path": "/v1/chat/completions",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"messages": [{"role": "user", "content": "hello"}]}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["model"], "facebook-suite")
        self.assertGreater(len(body["choices"]), 0)

    def test_upload_chunk_validation(self):
        req = {
            "method": "POST",
            "path": "/v1/upload/chunk",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 400)
        body = json.loads(resp["body"])
        self.assertIn("error", body)

    def test_upload_video_validation(self):
        req = {
            "method": "POST",
            "path": "/v1/upload/video",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 400)
        body = json.loads(resp["body"])
        self.assertIn("error", body)

    def test_not_found(self):
        req = {"method": "GET", "path": "/unknown-route", "headers": {}, "body": ""}
        resp = handle_request(req)
        self.assertEqual(resp["status"], 404)

    @mock.patch("adapter.publish_feed_status")
    def test_publish_status_route(self, mock_publish):
        mock_publish.return_value = {"id": "1435449429646177_status123"}
        req = {
            "method": "POST",
            "path": "/v1/publish/status",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"message": "Hello live!"}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertEqual(body["id"], "1435449429646177_status123")

    @mock.patch("adapter.upload_and_publish_video")
    def test_upload_video_with_video_path(self, mock_upload):
        mock_upload.return_value = {"status": "success", "video_id": "test_vid_123"}
        tmp_video = Path(self._tmp.name) / "test.mp4"
        tmp_video.write_bytes(b"dummy video content")
        req = {
            "method": "POST",
            "path": "/v1/upload/video",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({
                "video_path": str(tmp_video),
                "title": "My Reel",
                "schedule_time": 1793500000,
                "publish_type": "FEED",
            }),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        mock_upload.assert_called_once()
        args, kwargs = mock_upload.call_args
        self.assertEqual(kwargs.get("title"), "My Reel")
        self.assertEqual(kwargs.get("scheduled_publish_time"), 1793500000)
        self.assertEqual(kwargs.get("publish_type"), "FEED")

    @mock.patch("adapter.upload_and_publish_video")
    def test_upload_video_path_traversal_rejected(self, mock_upload):
        req = {
            "method": "POST",
            "path": "/v1/upload/video",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({
                "video_path": "/etc/passwd",
                "title": "Malicious",
            }),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 400)
        mock_upload.assert_not_called()

    @mock.patch("adapter.upload_and_publish_video")
    def test_one_step_publish_with_file_path(self, mock_upload):
        mock_upload.return_value = {"status": "success", "video_id": "test_vid_456"}
        tmp_video = Path(self._tmp.name) / "short.mp4"
        tmp_video.write_bytes(b"another dummy short")
        req = {
            "method": "POST",
            "path": "/v1/publish",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({
                "target_id": "1435449429646177",
                "file_path": str(tmp_video),
                "title": "One Step Short",
                "scheduled_publish_time": 1793600000,
            }),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        mock_upload.assert_called_once()
        args, kwargs = mock_upload.call_args
        self.assertEqual(kwargs.get("scheduled_publish_time"), 1793600000)

    @mock.patch("adapter.publish_feed_status")
    def test_publish_status_with_schedule_time(self, mock_publish):
        mock_publish.return_value = {"id": "1435449429646177_status456"}
        req = {
            "method": "POST",
            "path": "/v1/publish/status",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({
                "message": "Scheduled live status",
                "schedule_time": 1793700000,
            }),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        mock_publish.assert_called_once()
        args, kwargs = mock_publish.call_args
        self.assertEqual(kwargs.get("scheduled_publish_time"), 1793700000)

    @mock.patch("adapter.delete_object")
    def test_delete_route(self, mock_delete):
        mock_delete.return_value = {"success": True}
        req = {
            "method": "POST",
            "path": "/v1/delete",
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"object_id": "1435449429646177_status123"}),
        }
        resp = handle_request(req)
        self.assertEqual(resp["status"], 200)
        body = json.loads(resp["body"])
        self.assertTrue(body["success"])


if __name__ == "__main__":
    unittest.main()
