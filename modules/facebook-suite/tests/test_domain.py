"""
Unit tests for Meta Business Suite domain core.
"""

import os
from pathlib import Path
import unittest

from facebook_suite.domain import (
    DEFAULT_TRANSLATED_LANGUAGES,
    FacebookCredentials,
    PublishSpec,
    VideoUploadSession,
    build_rupload_headers,
    build_translated_audio_metadata,
    derive_jazoest,
    extract_facebook_config_from_har,
    mask_secret,
)


class TestFacebookDomain(unittest.TestCase):

    def test_derive_jazoest(self):
        token = "test_token_abc:1:1234567890"
        jazoest = derive_jazoest(token)
        self.assertEqual(jazoest, "22167")
        self.assertEqual(derive_jazoest(""), "")

    def test_mask_secret(self):
        self.assertEqual(mask_secret(None), "not set")
        self.assertEqual(mask_secret(""), "not set")
        self.assertEqual(mask_secret("12345"), "***")
        masked = mask_secret("123456789", 4, 3)
        self.assertTrue(masked.startswith("1234..."))
        self.assertTrue(masked.endswith("789"))

    def test_extract_facebook_config_from_har_synthetic(self):
        sample_har = {
            "log": {
                "entries": [
                    {
                        "request": {
                            "url": "https://business.facebook.com/ajax/video/upload/requests/start/?av=987654321",
                            "method": "POST",
                            "headers": [
                                {"name": "Cookie", "value": "c_user=123456789; xs=abcd1234; datr=xyz987"},
                                {"name": "User-Agent", "value": "Mozilla/5.0 TestBrowser"},
                            ],
                            "postData": {
                                "text": "waterfall_id=test-uuid&target_id=987654321&__user=123456789&__bid=111222333&fb_dtsg=test_token_abc%3A1%3A1234567890&jazoest=22167&lsd=test_lsd_value"
                            },
                        }
                    }
                ]
            }
        }
        extracted = extract_facebook_config_from_har(sample_har)
        self.assertEqual(extracted["user_id"], "123456789")
        self.assertEqual(extracted["actor_id"], "987654321")
        self.assertEqual(extracted["target_id"], "987654321")
        self.assertEqual(extracted["business_id"], "111222333")
        self.assertEqual(extracted["jazoest"], "22167")
        self.assertEqual(extracted["lsd"], "test_lsd_value")
        self.assertIn("c_user", extracted["cookie"])
        self.assertEqual(extracted["user_agent"], "Mozilla/5.0 TestBrowser")

    def test_extract_ignores_third_party_hosts(self):
        sample_har = {
            "log": {
                "entries": [
                    {
                        "request": {
                            "url": "https://analytics.thirdparty.com/track",
                            "method": "POST",
                            "headers": [
                                {"name": "Cookie", "value": "third_party_cookie=evil;"},
                                {"name": "User-Agent", "value": "EvilBrowser/1.0"},
                            ],
                            "postData": {
                                "text": "target_id=evil_id&__user=evil_user&fb_dtsg=evil_token"
                            },
                        }
                    },
                    {
                        "request": {
                            "url": "https://business.facebook.com/api/graphql/",
                            "method": "POST",
                            "headers": [
                                {"name": "Cookie", "value": "c_user=123456789; xs=valid_token;"},
                                {"name": "User-Agent", "value": "ValidBrowser/1.0"},
                            ],
                            "postData": {
                                "text": "target_id=987654321&__user=123456789&fb_dtsg=valid_dtsg"
                            },
                        }
                    }
                ]
            }
        }
        extracted = extract_facebook_config_from_har(sample_har)
        self.assertEqual(extracted["user_id"], "123456789")
        self.assertEqual(extracted["target_id"], "987654321")
        self.assertEqual(extracted["cookie"], "c_user=123456789; xs=valid_token;")
        self.assertEqual(extracted["user_agent"], "ValidBrowser/1.0")

    def test_extract_from_real_har_file(self):
        har_path = Path(__file__).resolve().parent.parent.parent.parent / "target" / "download-facebook.har"
        if not har_path.is_file():
            self.skipTest("target/download-facebook.har not present")

        with open(har_path, "r", encoding="utf-8") as f:
            raw = f.read()
        extracted = extract_facebook_config_from_har(raw)

        self.assertTrue(extracted["user_id"])
        self.assertTrue(extracted["actor_id"])
        self.assertTrue(extracted["target_id"])
        self.assertTrue(extracted["fb_dtsg"])
        self.assertTrue(extracted["jazoest"])
        self.assertTrue(extracted["lsd"])

    def test_build_rupload_headers(self):
        session = VideoUploadSession(
            waterfall_id="test-waterfall",
            video_id="video-123",
            upload_session_id="session-456",
            file_size=2048,
            file_name="clip.mp4",
        )
        headers = build_rupload_headers(session, chunk_length=1024, offset=0, user_agent="CustomUA")
        self.assertEqual(headers["Offset"], "0")
        self.assertEqual(headers["X-Entity-Length"], "2048")
        self.assertEqual(headers["X-Entity-Name"], "clip.mp4")
        self.assertEqual(headers["X-Entity-Type"], "video/mp4")
        self.assertEqual(headers["Content-Length"], "1024")
        self.assertEqual(headers["composer_session_id"], "test-waterfall")
        self.assertEqual(headers["id"], "session-456")
        self.assertEqual(headers["product_media_id"], "video-123")
        self.assertEqual(headers["User-Agent"], "CustomUA")

    def test_build_translated_audio_metadata_default(self):
        meta = build_translated_audio_metadata()
        self.assertEqual(len(meta), 14)
        self.assertEqual(len(meta), len(DEFAULT_TRANSLATED_LANGUAGES))
        for item in meta:
            self.assertTrue(item["enabled"])
            self.assertFalse(item["is_lipsync_enabled"])
            self.assertFalse(item["require_review"])
            self.assertIn(item["lang_code"], DEFAULT_TRANSLATED_LANGUAGES)

    def test_build_translated_audio_metadata_custom(self):
        custom_langs = ["es_LA", "fr_FR", "de_DE"]
        meta = build_translated_audio_metadata(enabled=False, languages=custom_langs)
        self.assertEqual(len(meta), 3)
        self.assertEqual([m["lang_code"] for m in meta], custom_langs)
        for item in meta:
            self.assertFalse(item["enabled"])

    def test_publish_spec_auto_translate_defaults(self):
        spec = PublishSpec(target_id="123", video_id="456")
        self.assertTrue(spec.enable_auto_translate)
        self.assertIsNone(spec.translated_languages)


if __name__ == "__main__":
    unittest.main()
