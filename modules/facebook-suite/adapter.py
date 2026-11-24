#!/usr/bin/env python3
"""
Meta Business Suite / Facebook Creator Studio Adapter for Toolcase Gateway (stdio-stream protocol).
Clean Architecture Layer: Interface Adapters.
"""

import base64
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from typing import Optional, Tuple
import urllib.parse
import uuid

from facebook_suite import (
    PublishSpec,
    VideoUploadSession,
    build_masked_config_view,
    delete_object,
    extract_facebook_config_from_har,
    fetch_page_access_token,
    fetch_profile_or_scopes,
    finish_video_upload,
    init_video_upload,
    load_module_config,
    publish_feed_status,
    publish_reel_or_post,
    render_module_ui_html,
    save_module_config,
    start_video_upload,
    upload_and_publish_video,
    upload_video_chunk,
)


def json_resp(status: int, payload: dict) -> dict:
    return {
        "status": status,
        "headers": {"content-type": "application/json"},
        "body": json.dumps(payload),
    }


def html_resp(status: int, html: str) -> dict:
    return {
        "status": status,
        "headers": {"content-type": "text/html; charset=utf-8"},
        "body": html,
    }


def normalize_headers(raw_headers) -> dict:
    if isinstance(raw_headers, dict):
        return {str(k).lower(): str(v) for k, v in raw_headers.items()}
    if isinstance(raw_headers, list):
        out = {}
        for item in raw_headers:
            if isinstance(item, (list, tuple)) and len(item) == 2:
                out[str(item[0]).lower()] = str(item[1])
        return out
    return {}


def _handle_api_resp(res: dict, success_code: int = 200) -> dict:
    """Inspects upstream API result and returns appropriate HTTP status code."""
    if isinstance(res, dict) and "error" in res:
        err = res["error"]
        code = 502
        if isinstance(err, dict) and isinstance(err.get("code"), int) and 400 <= err["code"] < 600:
            code = err["code"]
        return json_resp(code, res)
    return json_resp(success_code, res)


def _is_safe_video_path(p: Path) -> bool:
    """Verifies that the target path stays strictly within allowed directories (no path traversal)."""
    try:
        resolved = p.resolve(strict=True)
        allowed_roots = [
            Path(tempfile.gettempdir()).resolve(),
            Path("/tmp").resolve(),
            Path.cwd().resolve(),
        ]
        media_env = os.environ.get("MEDIA_DIR")
        if media_env:
            allowed_roots.append(Path(media_env).resolve())
        return any(resolved == root or resolved.is_relative_to(root) for root in allowed_roots)
    except (ValueError, RuntimeError, OSError):
        return False


def _extract_video_bytes(body: dict) -> Tuple[Optional[bytes], str]:
    """Extracts raw video bytes and filename from video_path, file_path, or video_base64."""
    video_bytes = None
    file_name = str(body.get("file_name") or "")
    if "video_path" in body or "file_path" in body:
        path_str = str(body.get("video_path") or body.get("file_path"))
        if ".." not in path_str and not path_str.startswith("\x00"):
            p = Path(path_str)
            if p.is_file() and _is_safe_video_path(p):
                video_bytes = p.read_bytes()
                if not file_name:
                    file_name = p.name
    elif "video_base64" in body and body["video_base64"]:
        video_bytes = base64.b64decode(str(body["video_base64"]))
    return video_bytes, file_name or "video.mp4"


def _extract_schedule_time(body: dict) -> Optional[int]:
    """Extracts scheduled timestamp, supporting schedule_time alias."""
    val = body.get("scheduled_publish_time")
    if val is None:
        val = body.get("schedule_time")
    if val is not None:
        try:
            return int(val)
        except (ValueError, TypeError):
            pass
    return None


def handle_request(req: dict) -> dict:
    method = str(req.get("method") or "GET").upper()
    raw_path = str(req.get("path") or "/")
    clean_path, _, _ = raw_path.partition("?")
    sub = clean_path.strip("/")
    headers = normalize_headers(req.get("headers"))
    raw_body = str(req.get("body") or "")
    namespace = os.environ.get("MODULE_NAMESPACE", "/custom/facebook")
    module_id = os.environ.get("MODULE_ID", "facebook-suite")
    cfg = load_module_config()

    ns_stripped = namespace.strip("/")
    if ns_stripped and sub.startswith(ns_stripped):
        sub = sub[len(ns_stripped):].strip("/")
    elif sub.startswith("facebook/"):
        sub = sub[len("facebook/"):].strip("/")

    # Enforce enabled flag on active upstream operational endpoints
    if not cfg.get("enabled", True) and sub not in (
        "", "ui", "settings", "config-page", "v1/config-page", "studio/ui",
        "status", "v1/status", "studio/status",
        "health", "v1/health", "studio/health",
        "config", "v1/config", "studio/config",
        "config/har", "v1/config/har", "studio/config/har",
    ) and not req.get("probe"):
        return json_resp(403, {"error": {"code": 403, "message": "Meta Business Suite module is currently disabled."}})

    # 1. UI Configuration Page
    if method == "GET" and sub in ("", "ui", "settings", "config-page", "v1/config-page", "studio/ui"):
        return html_resp(200, render_module_ui_html(namespace))

    # 2. Health & Diagnostic Probe Endpoint
    is_probe = bool(req.get("probe"))
    if method == "GET" and (sub in ("health", "v1/health", "studio/health") or is_probe):
        masked = build_masked_config_view(cfg)
        return json_resp(200, {
            "status": "healthy",
            "ok": True,
            "healthy": True,
            "module": module_id,
            "enabled": cfg.get("enabled", True),
            "configured": masked.get("configured", False),
            "namespace": namespace,
            **masked,
        })

    # 3. Status Endpoint
    if method == "GET" and sub in ("status", "v1/status", "studio/status"):
        masked = build_masked_config_view(cfg)
        return json_resp(200, {
            "module": module_id,
            "status": "enabled" if cfg.get("enabled", True) else "disabled",
            "ok": True,
            "healthy": cfg.get("enabled", True),
            "namespace": namespace,
            **masked,
        })

    # 4. Config Read/Write
    if sub in ("config", "v1/config", "studio/config"):
        if method == "GET":
            return json_resp(200, {"status": "ok", **build_masked_config_view(cfg)})
        if method == "POST":
            try:
                body_json = json.loads(raw_body) if raw_body else {}
                updated = save_module_config(body_json)
                return json_resp(200, {
                    "status": "ok",
                    "message": "Meta Business Suite configuration updated and saved.",
                    **build_masked_config_view(updated),
                })
            except Exception as exc:
                return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    # 5. HAR Import Endpoint
    if method == "POST" and sub in ("config/har", "v1/config/har", "studio/config/har"):
        try:
            extracted = extract_facebook_config_from_har(raw_body)
            updated = save_module_config(extracted)
            return json_resp(
                200,
                {
                    "status": "ok",
                    "message": "HAR credentials and Comet parameters extracted and saved.",
                    **build_masked_config_view(updated),
                },
            )
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    # 5. Scopes and Identities
    if sub in ("scopes", "v1/scopes", "user", "v1/user", "studio/user"):
        try:
            return _handle_api_resp(fetch_profile_or_scopes(cfg, headers))
        except Exception as exc:
            return json_resp(500, {"error": {"code": 500, "message": str(exc)}})

    # 6. Video Upload Pipeline
    if sub in ("upload/init", "v1/upload/init") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            file_size = int(b.get("file_size", 0))
            file_ext = str(b.get("file_extension", "mp4"))
            target_id = b.get("target_id")
            return _handle_api_resp(init_video_upload(cfg, file_size, file_ext, target_id))
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("upload/start", "v1/upload/start") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            session = VideoUploadSession(
                waterfall_id=str(b.get("waterfall_id") or uuid.uuid4()),
                file_size=int(b.get("file_size", 0)),
                file_name=b.get("file_name", "video.mp4"),
            )
            res = start_video_upload(cfg, session, b.get("target_id"))
            if isinstance(res, dict) and "error" not in res:
                res = {**res, "waterfall_id": session.waterfall_id}
            return _handle_api_resp(res)
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("upload/chunk", "v1/upload/chunk") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            chunk_b64 = b.get("chunk_base64") or b.get("data")
            if not chunk_b64:
                return json_resp(400, {"error": {"code": 400, "message": "Missing chunk_base64 or data"}})
            chunk_bytes = base64.b64decode(chunk_b64)
            offset = int(b.get("offset", 0))
            session = VideoUploadSession(
                waterfall_id=str(b.get("waterfall_id") or ""),
                video_id=str(b.get("video_id") or ""),
                upload_session_id=str(b.get("upload_session_id") or ""),
                file_size=int(b.get("file_size", len(chunk_bytes))),
            )
            res = upload_video_chunk(cfg, session, chunk_bytes, offset)
            return _handle_api_resp(res)
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("upload/video", "v1/upload/video") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            video_bytes, file_name = _extract_video_bytes(b)
            if not video_bytes:
                return json_resp(400, {"error": {"code": 400, "message": "Missing video_base64 or valid video_path"}})
            auto_trans = b.get("enable_auto_translate")
            if auto_trans is None:
                auto_trans = cfg.get("auto_translate", True)
            res = upload_and_publish_video(
                cfg,
                video_bytes,
                file_name=file_name,
                title=str(b.get("title", "New Reel")),
                description=str(b.get("description", "")),
                target_id=b.get("target_id"),
                scheduled_publish_time=_extract_schedule_time(b),
                enable_auto_translate=auto_trans,
                translated_languages=b.get("translated_languages"),
                publish_type=str(b.get("publish_type", "REEL")),
            )
            if res.get("status") == "error":
                return json_resp(502, res)
            return json_resp(200, res)
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("upload/finish", "v1/upload/finish") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            waterfall_id = str(b.get("waterfall_id") or "")
            if not waterfall_id:
                return json_resp(400, {"error": {"code": 400, "message": "Missing required waterfall_id"}})
            session = VideoUploadSession(
                waterfall_id=waterfall_id,
                video_id=b.get("video_id", ""),
                file_size=int(b.get("file_size", 0)),
                start_offset=int(b.get("start_offset", 0)),
            )
            res = finish_video_upload(cfg, session, b.get("target_id"))
            return _handle_api_resp(res)
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    # 7. Status, Feed & Object Publishing
    if sub in ("publish/status", "v1/publish/status", "feed/post") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            msg = b.get("message") or b.get("text") or b.get("content")
            if not msg:
                return json_resp(400, {"error": {"code": 400, "message": "Missing message or text"}})
            sched_time = _extract_schedule_time(b)
            return _handle_api_resp(publish_feed_status(cfg, msg, b.get("target_id"), scheduled_publish_time=sched_time))
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("delete", "v1/delete", "publish/delete") and method in ("POST", "DELETE"):
        try:
            b = json.loads(raw_body) if raw_body else {}
            obj_id = b.get("object_id") or b.get("id") or b.get("post_id") or b.get("video_id")
            if not obj_id:
                return json_resp(400, {"error": {"code": 400, "message": "Missing object_id or id"}})
            return _handle_api_resp(delete_object(cfg, obj_id))
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    if sub in ("token", "v1/token", "access_token") and method == "GET":
        tok = fetch_page_access_token(cfg)
        if tok:
            return json_resp(200, {"status": "ok", "access_token": tok[:10] + "..." + tok[-5:], "discovered": True})
        return json_resp(404, {"error": {"code": 404, "message": "No active page token discovered."}})

    # 8. Reel, Story & Post Publish (1-step video upload & publish, or video_id / text status publish)
    if sub in ("publish", "v1/publish", "reels", "v1/reels") and method == "POST":
        try:
            b = json.loads(raw_body) if raw_body else {}
            target_id = b.get("target_id") or cfg.get("target_id") or cfg.get("actor_id")
            if not target_id:
                return json_resp(400, {"error": {"code": 400, "message": "Missing target_id"}})
            auto_trans = b.get("enable_auto_translate")
            if auto_trans is None:
                auto_trans = cfg.get("auto_translate", True)
            sched_time = _extract_schedule_time(b)
            # 1-step direct video file or base64 publish
            video_bytes, file_name = _extract_video_bytes(b)
            if video_bytes:
                res = upload_and_publish_video(
                    cfg,
                    video_bytes,
                    file_name=file_name,
                    title=str(b.get("title", "New Reel")),
                    description=str(b.get("description", "")),
                    target_id=target_id,
                    scheduled_publish_time=sched_time,
                    enable_auto_translate=auto_trans,
                    translated_languages=b.get("translated_languages"),
                    publish_type=str(b.get("publish_type", "REEL")),
                )
                if res.get("status") == "error":
                    return json_resp(502, res)
                return json_resp(200, res)
            video_id = b.get("video_id", "")
            if not video_id:
                msg = b.get("message") or b.get("text") or b.get("description")
                if msg:
                    return _handle_api_resp(publish_feed_status(cfg, msg, target_id, scheduled_publish_time=sched_time))
                return json_resp(400, {"error": {"code": 400, "message": "Missing video_id, video_path, or message"}})
            spec = PublishSpec(
                target_id=target_id,
                video_id=video_id,
                title=b.get("title", ""),
                description=b.get("description", ""),
                publish_type=b.get("publish_type", "REEL"),
                scheduled_publish_time=sched_time,
                enable_auto_translate=auto_trans,
                translated_languages=b.get("translated_languages"),
            )
            return _handle_api_resp(publish_reel_or_post(cfg, spec))
        except Exception as exc:
            return json_resp(400, {"error": {"code": 400, "message": str(exc)}})

    # 8. OpenAI Models facade
    if method == "GET" and sub in ("models", "v1/models"):
        return json_resp(
            200,
            {
                "object": "list",
                "data": [
                    {
                        "id": "facebook-suite",
                        "object": "model",
                        "created": int(time.time()),
                        "owned_by": "meta-business-suite",
                    }
                ],
            },
        )

    # 9. OpenAI Chat Completions facade
    if method == "POST" and sub in ("chat/completions", "v1/chat/completions"):
        masked = build_masked_config_view(cfg)
        status_desc = f"Meta Business Suite Adapter is active. Actor: {masked['actor_id']}, Page: {masked['target_id']}."
        return json_resp(
            200,
            {
                "id": f"chatcmpl-fb-{uuid.uuid4().hex[:8]}",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": "facebook-suite",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": status_desc,
                        },
                        "finish_reason": "stop",
                    }
                ],
            },
        )

    return json_resp(404, {"error": {"code": 404, "message": f"Endpoint not found: {sub}"}})


def main():
    line = sys.stdin.readline()
    if not line:
        return
    try:
        req = json.loads(line)
        resp = handle_request(req)
    except Exception as exc:
        resp = {
            "status": 500,
            "headers": {"content-type": "application/json"},
            "body": json.dumps({"error": {"code": 500, "message": str(exc)}}),
        }
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
