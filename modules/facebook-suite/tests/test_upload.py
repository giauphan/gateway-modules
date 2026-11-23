"""
Unit tests for Meta Business Suite upload and publish pipeline with Mock Transport.
"""

from typing import Any, Dict
import unittest
from unittest import mock

from facebook_suite.domain import PublishSpec, VideoUploadSession
from facebook_suite.ports import FacebookTransportPort, FacebookUploadUseCasePort
from facebook_suite.upload import (
    DEFAULT_CHUNK_SIZE,
    FacebookUploadUseCase,
    publish_reel_or_post,
    upload_and_publish_video,
)


class MockFacebookTransport(FacebookTransportPort):
    """In-memory mock transport for unit testing without live network."""

    def __init__(self):
        self.calls = []

    def post_form(
        self,
        url: str,
        form_data: Dict[str, Any],
        headers: Dict[str, str],
        timeout: int = 30,
    ) -> Dict[str, Any]:
        self.calls.append(("post_form", url, form_data))
        if "init_upload" in url:
            return {"payload": {"videoUploader": [{"start_uri": "mock"}]}}
        if "requests/start" in url:
            return {
                "payload": {
                    "video_id": "mock_video_123",
                    "upload_session_id": "mock_session_456",
                    "start_offset": 0,
                    "end_offset": 1048576,
                    "skip_upload": False,
                }
            }
        if "requests/receive" in url:
            return {"payload": {"start_offset": 1024, "end_offset": 1024}}
        if "api/graphql" in url:
            return {"data": {"story_create": {"id": "story_mock_789"}}}
        return {"status": "ok"}

    def post_binary(
        self,
        url: str,
        data: bytes,
        headers: Dict[str, str],
        timeout: int = 60,
    ) -> Dict[str, Any]:
        self.calls.append(("post_binary", url, len(data)))
        return {"status": 200, "bytes_written": len(data)}


class TestFacebookUpload(unittest.TestCase):

    def test_upload_session_progress(self):
        session = VideoUploadSession(
            waterfall_id="test-waterfall-123",
            video_id="video-987",
            upload_session_id="session-654",
            start_offset=0,
            end_offset=0,
            file_size=2000000,
        )
        self.assertFalse(session.is_complete)
        session.end_offset = 1000000
        self.assertFalse(session.is_complete)
        session.end_offset = 2000000
        self.assertTrue(session.is_complete)

    def test_publish_spec(self):
        spec = PublishSpec(
            target_id="987654321",
            video_id="video-987",
            title="My New Reel",
            description="Testing reel description",
            publish_type="REEL",
        )
        self.assertEqual(spec.target_id, "987654321")
        self.assertEqual(spec.video_id, "video-987")
        self.assertEqual(spec.publish_type, "REEL")
        self.assertEqual(spec.title, "My New Reel")

    def test_chunking_offsets(self):
        total_size = 2500000
        chunk_size = DEFAULT_CHUNK_SIZE  # 1 MB = 1048576
        offsets = []
        offset = 0
        while offset < total_size:
            chunk_len = min(chunk_size, total_size - offset)
            offsets.append((offset, offset + chunk_len))
            offset += chunk_len

        self.assertEqual(len(offsets), 3)
        self.assertEqual(offsets[0], (0, 1048576))
        self.assertEqual(offsets[1], (1048576, 2097152))
        self.assertEqual(offsets[2], (2097152, 2500000))

    def test_mock_transport_upload_and_publish_pipeline(self):
        mock_tp = MockFacebookTransport()
        cfg = {
            "base_url": "https://business.facebook.com",
            "actor_id": "987654321",
            "target_id": "987654321",
            "user_id": "123456789",
            "fb_dtsg": "mock_token",
        }
        dummy_video = b"MOCK_MP4_HEADER" + b"\x00" * 2048

        use_case = FacebookUploadUseCase(cfg=cfg, transport=mock_tp)
        self.assertIsInstance(use_case, FacebookUploadUseCasePort)

        res = use_case.upload_and_publish(
            video_bytes=dummy_video,
            file_name="test_reel.mp4",
            title="Clean Architecture Reel",
            description="Testing DIP with Mock Transport",
        )

        self.assertEqual(res["status"], "success")
        self.assertEqual(res["video_id"], "mock_video_123")
        self.assertEqual(res["target_id"], "987654321")
        self.assertIn("data", res["publish_result"])

        # Check call trace
        action_names = [call[0] for call in mock_tp.calls]
        self.assertIn("post_form", action_names)
        self.assertIn("post_binary", action_names)

    @mock.patch("urllib.request.urlopen")
    def test_upload_video_graph_fastpath(self, mock_urlopen):
        import json
        mock_resp = mock.MagicMock()
        mock_resp.read.return_value = json.dumps({"id": "999888777"}).encode("utf-8")
        mock_urlopen.return_value.__enter__.return_value = mock_resp

        cfg = {
            "actor_id": "987654321",
            "target_id": "987654321",
            "access_token": "EAAtest_token",
        }
        res = upload_and_publish_video(
            cfg,
            b"FAKE_VIDEO_BYTES",
            file_name="test.mp4",
            title="Graph Fastpath",
            description="Testing graph video upload",
        )
        self.assertEqual(res["status"], "success")
        self.assertEqual(res["video_id"], "999888777")
        self.assertEqual(res["publish_result"]["id"], "999888777")

    def test_publish_reel_or_post_includes_auto_translate_by_default(self):
        import json
        mock_tp = MockFacebookTransport()
        cfg = {"base_url": "https://business.facebook.com", "actor_id": "987", "user_id": "123", "fb_dtsg": "tok"}
        spec = PublishSpec(target_id="987", video_id="vid-101", title="Translate Video")

        res = publish_reel_or_post(cfg, spec, transport=mock_tp)
        self.assertIn("data", res)

        form_call = next(c for c in mock_tp.calls if c[0] == "post_form")
        form_data = form_call[2]
        variables = json.loads(form_data["variables"])
        base = variables["input"]["base"]

        self.assertIn("attachments", base)
        att = base["attachments"][0]["video"]
        self.assertEqual(att["id"], "vid-101")
        tam = att["additional_video_metadata"]["translatedAudioMetadata"]
        self.assertEqual(len(tam), 14)
        for entry in tam:
            self.assertTrue(entry["enabled"])
            self.assertIn("lang_code", entry)

    def test_publish_reel_or_post_auto_translate_disabled(self):
        import json
        mock_tp = MockFacebookTransport()
        cfg = {"base_url": "https://business.facebook.com", "actor_id": "987", "user_id": "123", "fb_dtsg": "tok"}
        spec = PublishSpec(target_id="987", video_id="vid-102", enable_auto_translate=False)

        res = publish_reel_or_post(cfg, spec, transport=mock_tp)
        self.assertIn("data", res)

        form_call = next(c for c in mock_tp.calls if c[0] == "post_form")
        variables = json.loads(form_call[2]["variables"])
        base = variables["input"]["base"]
        self.assertNotIn("attachments", base)

    def test_publish_reel_or_post_custom_languages(self):
        import json
        mock_tp = MockFacebookTransport()
        cfg = {"base_url": "https://business.facebook.com", "actor_id": "987", "user_id": "123", "fb_dtsg": "tok"}
        spec = PublishSpec(
            target_id="987",
            video_id="vid-103",
            enable_auto_translate=True,
            translated_languages=["vi_VN", "th_TH"],
        )

        res = publish_reel_or_post(cfg, spec, transport=mock_tp)
        self.assertIn("data", res)

        form_call = next(c for c in mock_tp.calls if c[0] == "post_form")
        variables = json.loads(form_call[2]["variables"])
        tam = variables["input"]["base"]["attachments"][0]["video"]["additional_video_metadata"]["translatedAudioMetadata"]
        self.assertEqual(len(tam), 2)
        self.assertEqual([e["lang_code"] for e in tam], ["vi_VN", "th_TH"])


if __name__ == "__main__":
    unittest.main()
