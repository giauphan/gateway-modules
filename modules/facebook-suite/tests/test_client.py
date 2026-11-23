"""
Unit tests for Meta Business Suite application services.
"""

import io
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import facebook_suite.client as client
from facebook_suite.client import (
    build_authenticated_headers,
    build_masked_config_view,
    fetch_profile_or_scopes,
    load_module_config,
    save_module_config,
)


class TestFacebookClient(unittest.TestCase):

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        p = mock.patch.object(client, "get_config_path", return_value=Path(self._tmp.name) / "config.json")
        p.start()
        self.addCleanup(p.stop)
        self.addCleanup(self._tmp.cleanup)
        client._RUNTIME_CONFIG_CACHE = None
        self.addCleanup(setattr, client, "_RUNTIME_CONFIG_CACHE", None)

    def test_build_masked_config_view(self):
        cfg = {
            "base_url": "https://business.facebook.com",
            "enabled": True,
            "user_id": "123456789",
            "actor_id": "987654321",
            "target_id": "987654321",
            "business_id": "111222333",
            "fb_dtsg": "test_token_abc123:1:1234567890",
            "cookie": "c_user=123456789; xs=test_secret_xs;",
            "user_agent": "TestBrowser/1.0",
        }
        masked = build_masked_config_view(cfg)
        self.assertTrue(masked["configured"])
        self.assertEqual(masked["base_url"], "https://business.facebook.com")
        self.assertTrue(masked["enabled"])
        self.assertTrue(masked["user_id"].startswith("1234..."))
        self.assertTrue(masked["masked_cookie"].startswith("c_user..."))
        self.assertNotIn("test_secret_xs", masked["masked_cookie"])
        self.assertNotIn("abc123:1:", masked["masked_fb_dtsg"])

    def test_save_module_config_validates_base_url(self):
        # Invalid base_url rejected
        with self.assertRaises(ValueError):
            save_module_config({"base_url": "http://evil.com"})
        with self.assertRaises(ValueError):
            save_module_config({"base_url": "https://notfacebook.com"})

        # Valid base_url accepted
        saved = save_module_config({"base_url": "https://business.facebook.com", "target_id": "12345"})
        self.assertEqual(saved["base_url"], "https://business.facebook.com")
        self.assertEqual(saved["target_id"], "12345")

    def test_build_authenticated_headers(self):
        cfg = {
            "cookie": "c_user=1000; xs=token;",
            "lsd": "testlsd",
            "user_agent": "Mozilla/5.0 Custom",
        }
        headers = build_authenticated_headers(cfg, {"X-Custom": "val"})
        self.assertEqual(headers["User-Agent"], "Mozilla/5.0 Custom")
        self.assertEqual(headers["Cookie"], "c_user=1000; xs=token;")
        self.assertEqual(headers["x-fb-lsd"], "testlsd")
        self.assertEqual(headers["X-Custom"], "val")
        self.assertEqual(headers["Origin"], "https://business.facebook.com")

    def test_build_authenticated_headers_formats_raw_xs(self):
        cfg = {
            "user_id": "100049522402352",
            "cookie": "42%3Awg8f7vd2wMarRA",
        }
        headers = build_authenticated_headers(cfg)
        self.assertEqual(headers["Cookie"], "c_user=100049522402352; xs=42%3Awg8f7vd2wMarRA;")

    @mock.patch("urllib.request.urlopen")
    def test_publish_feed_status_success(self, mock_urlopen):
        import io
        import json
        mock_resp = io.BytesIO(json.dumps({"id": "1435449429646177_12345"}).encode("utf-8"))
        mock_urlopen.return_value.__enter__.return_value = mock_resp
        
        cfg = {"target_id": "1435449429646177", "access_token": "EAAtest_token"}
        res = client.publish_feed_status(cfg, "Test message")
        self.assertEqual(res["id"], "1435449429646177_12345")

    @mock.patch("urllib.request.urlopen")
    def test_delete_object_success(self, mock_urlopen):
        import io
        import json
        mock_resp = io.BytesIO(json.dumps({"success": True}).encode("utf-8"))
        mock_urlopen.return_value.__enter__.return_value = mock_resp
        
        cfg = {"access_token": "EAAtest_token"}
        res = client.delete_object(cfg, "1435449429646177_12345")
        self.assertEqual(res["success"], True)

    def test_fetch_profile_or_scopes_fallback(self):
        cfg = {
            "user_id": "123456789",
            "actor_id": "987654321",
            "target_id": "987654321",
            "business_id": "111222333",
        }
        res = fetch_profile_or_scopes(cfg)
        self.assertEqual(res["status"], "ok")
        self.assertEqual(res["actor_id"], "987654321")
        self.assertEqual(res["default_target_id"], "987654321")
        self.assertEqual(len(res["scopes"]), 1)
        self.assertEqual(res["scopes"][0]["id"], "987654321")

    def test_default_config_auto_translate_is_true(self):
        cfg = client.load_module_config()
        self.assertIn("auto_translate", cfg)
        self.assertTrue(cfg["auto_translate"])

    def test_save_module_config_auto_translate_toggle(self):
        updated = client.save_module_config({"auto_translate": False})
        self.assertFalse(updated["auto_translate"])
        restored = client.save_module_config({"auto_translate": True})
        self.assertTrue(restored["auto_translate"])

    def test_save_module_config_auto_translate_invalid_type(self):
        with self.assertRaises(ValueError):
            client.save_module_config({"auto_translate": "not_a_bool"})

    @mock.patch("urllib.request.urlopen")
    def test_transport_html_redirect_returns_401(self, mock_urlopen):
        mock_resp = io.BytesIO(b'<!DOCTYPE html><html><title>Log in to Facebook</title><body>Login form</body></html>')
        mock_urlopen.return_value.__enter__.return_value = mock_resp
        tp = client.StandardFacebookTransport()
        res = tp.post_form("https://business.facebook.com/api", {"data": "test"}, {})
        self.assertIn("error", res)
        self.assertEqual(res["error"]["code"], 401)
        self.assertIn("expired or redirected to login", res["error"]["message"])

    @mock.patch.dict("os.environ", {"TOKEN_FACEBOOK": "EAABCTestEnvToken123"})
    def test_load_module_config_env_token(self):
        client._RUNTIME_CONFIG_CACHE = None
        cfg = client.load_module_config()
        self.assertEqual(cfg.get("access_token"), "EAABCTestEnvToken123")


if __name__ == "__main__":
    unittest.main()
