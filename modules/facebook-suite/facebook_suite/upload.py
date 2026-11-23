"""
Resumable chunked video upload pipeline and Reel publishing for Meta Business Suite.
Clean Architecture Layer: Application Use Cases & Pipeline Orchestration.
"""

import json
from typing import Any, Dict, List, Optional
import urllib.parse
import uuid

from .client import (
    StandardFacebookTransport,
    build_authenticated_headers,
    derive_jazoest,
    execute_graphql,
    fetch_page_access_token,
    load_module_config,
)
from .domain import (
    PublishSpec,
    VideoUploadSession,
    build_rupload_headers,
    build_translated_audio_metadata,
)
from .ports import FacebookTransportPort


DEFAULT_CHUNK_SIZE = 1024 * 1024  # 1 MB standard Facebook chunk


def init_video_upload(
    cfg: Dict[str, Any],
    file_size: int,
    file_extension: str = "mp4",
    target_id: Optional[str] = None,
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Step 1: Initializes composer video upload parameters with Meta Business Suite."""
    base_url = cfg.get("base_url", "https://business.facebook.com").rstrip("/")
    url = f"{base_url}/video/composer/init_upload/"

    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"error": {"code": 400, "message": "No target_id or actor_id configured."}}
    actor_id = cfg.get("actor_id") or tid
    user_id = cfg.get("user_id") or "0"

    form_data = {
        "target_id": tid,
        "av": actor_id,
        "__user": user_id,
        "__a": "1",
        "composer_entry_point_ref": "biz_web_home",
        "video_publisher_action_source[product]": "business_suite",
        "video_publisher_action_source[feature]": "cs_reel_composer",
        "video_publisher_action_source[entry_point]": "biz_web_home",
    }
    if cfg.get("fb_dtsg"):
        form_data["fb_dtsg"] = cfg["fb_dtsg"]
        form_data["jazoest"] = cfg.get("jazoest") or derive_jazoest(cfg["fb_dtsg"])

    return _post_form_request(url, form_data, cfg, transport)


def start_video_upload(
    cfg: Dict[str, Any],
    session: VideoUploadSession,
    target_id: Optional[str] = None,
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Step 2: Starts the video upload session and requests initial offset."""
    base_url = cfg.get("base_url", "https://business.facebook.com").rstrip("/")
    url = f"{base_url}/ajax/video/upload/requests/start/"

    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"error": {"code": 400, "message": "No target_id or actor_id configured."}}
    actor_id = cfg.get("actor_id") or tid
    user_id = cfg.get("user_id") or "0"

    form_data = {
        "waterfall_id": session.waterfall_id,
        "target_id": tid,
        "source": "composer_cs_reel_composer",
        "composer_entry_point_ref": "biz_web_home",
        "file_size": str(session.file_size),
        "file_extension": session.file_name.rsplit(".", 1)[-1],
        "supports_chunking": "true",
        "supports_file_api": "true",
        "av": actor_id,
        "__user": user_id,
        "__a": "1",
    }
    if cfg.get("fb_dtsg"):
        form_data["fb_dtsg"] = cfg["fb_dtsg"]
        form_data["jazoest"] = cfg.get("jazoest") or derive_jazoest(cfg["fb_dtsg"])

    res = _post_form_request(url, form_data, cfg, transport)
    payload = res.get("payload", {})
    if payload:
        session.video_id = str(payload.get("video_id", ""))
        session.upload_session_id = str(payload.get("upload_session_id", ""))
        session.start_offset = int(payload.get("start_offset", 0))
        session.end_offset = int(payload.get("end_offset", 0))
        session.skip_upload = bool(payload.get("skip_upload", False))

    return res


def upload_video_chunk(
    cfg: Dict[str, Any],
    session: VideoUploadSession,
    chunk_bytes: bytes,
    offset: int,
    rupload_domain: str = "rupload.facebook.com",
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Step 3: Uploads a binary video chunk to Meta rupload service."""
    upload_id = session.upload_session_id or session.video_id
    url = f"https://{rupload_domain}/fb_video/{upload_id}"

    user_id = cfg.get("user_id") or "0"
    actor_id = cfg.get("actor_id") or user_id
    query_params = {
        "__user": user_id,
        "av": actor_id,
        "__a": "1",
    }
    if cfg.get("fb_dtsg"):
        query_params["fb_dtsg"] = cfg["fb_dtsg"]
        query_params["jazoest"] = cfg.get("jazoest") or derive_jazoest(cfg["fb_dtsg"])

    full_url = f"{url}?{urllib.parse.urlencode(query_params)}"
    headers = build_rupload_headers(session, len(chunk_bytes), offset, cfg.get("user_agent"))

    tp = transport or StandardFacebookTransport()
    res = tp.post_binary(full_url, chunk_bytes, headers, timeout=60)
    if "error" not in res:
        session.end_offset = offset + len(chunk_bytes)
        return {"status": 200, "bytes_uploaded": len(chunk_bytes), "offset": session.end_offset}
    return res


def finish_video_upload(
    cfg: Dict[str, Any],
    session: VideoUploadSession,
    target_id: Optional[str] = None,
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Step 4: Commits and finalizes the video upload with Meta Business Suite."""
    base_url = cfg.get("base_url", "https://business.facebook.com").rstrip("/")
    url = f"{base_url}/ajax/video/upload/requests/receive/"

    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"error": {"code": 400, "message": "No target_id or actor_id configured."}}
    actor_id = cfg.get("actor_id") or tid
    user_id = cfg.get("user_id") or "0"

    form_data = {
        "waterfall_id": session.waterfall_id,
        "target_id": tid,
        "video_id": session.video_id,
        "source": "composer_cs_reel_composer",
        "composer_entry_point_ref": "biz_web_home",
        "supports_chunking": "true",
        "supports_upload_service": "true",
        "start_offset": str(session.start_offset),
        "end_offset": str(session.file_size),
        "av": actor_id,
        "__user": user_id,
        "__a": "1",
    }
    if cfg.get("fb_dtsg"):
        form_data["fb_dtsg"] = cfg["fb_dtsg"]
        form_data["jazoest"] = cfg.get("jazoest") or derive_jazoest(cfg["fb_dtsg"])

    return _post_form_request(url, form_data, cfg, transport)


def publish_reel_or_post(
    cfg: Dict[str, Any],
    spec: PublishSpec,
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """
    Step 5: Publishes the uploaded video as a Reel, Story, or Post via GraphQL.
    Mutation: BusinessComposerStoryCreationMutation (doc_id: 9453776261410796)
    """
    pt = (spec.publish_type or "REEL").upper()
    if pt in ("REEL", "REELS"):
        channel_type = "FACEBOOK_REELS"
        creation_source = "cs_reel_composer"
    elif pt in ("STORY", "STORIES"):
        channel_type = "FACEBOOK_STORIES"
        creation_source = "cs_story_composer"
    elif pt in ("FEED", "POST", "NEWS_FEED"):
        channel_type = "FACEBOOK_NEWS_FEED"
        creation_source = "cs_feed_composer"
    else:
        return {
            "error": {
                "code": 400,
                "message": f"Unsupported publish_type '{spec.publish_type}'. Supported: REEL, STORY, FEED",
            }
        }

    doc_id = "9453776261410796"
    mutation_id = spec.client_mutation_id or str(uuid.uuid4())

    variables = {
        "input": {
            "client_mutation_id": mutation_id,
            "base": {
                "composer_entry_point_ref": "biz_web_home",
                "creation_source": creation_source,
                "message": {"text": spec.description or spec.title},
            },
            "channels": [
                {
                    "channel_type": channel_type,
                    "video_channel_spec": {
                        "video_id": spec.video_id,
                        "title": spec.title,
                        "video_target_spec": {
                            "target_id": spec.target_id,
                            "target_type": "PAGE",
                        },
                    },
                }
            ],
            "identities": [{"actor_id": spec.target_id}],
        }
    }

    if spec.enable_auto_translate:
        variables["input"]["base"]["attachments"] = [
            {
                "video": {
                    "id": spec.video_id,
                    "additional_video_metadata": {
                        "translatedAudioMetadata": build_translated_audio_metadata(
                            True, spec.translated_languages
                        ),
                    },
                }
            }
        ]

    if spec.scheduled_publish_time:
        variables["input"]["base"]["publish_time"] = spec.scheduled_publish_time

    return execute_graphql(
        cfg,
        "BusinessComposerStoryCreationMutation",
        doc_id,
        variables,
        transport=transport,
    )


def upload_video_graph(
    cfg: Dict[str, Any],
    video_bytes: bytes,
    file_name: str = "video.mp4",
    title: str = "New Video",
    description: str = "",
    target_id: Optional[str] = None,
    page_token: Optional[str] = None,
    scheduled_publish_time: Optional[int] = None,
) -> Dict[str, Any]:
    """Uploads and publishes video directly via Facebook Graph Video API."""
    import urllib.request
    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"error": {"code": 400, "message": "No target_id configured."}}

    token = page_token or cfg.get("access_token") or fetch_page_access_token(cfg)
    if not token:
        return {"error": {"code": 401, "message": "Could not acquire valid Page Access Token."}}

    boundary = "----WebKitFormBoundary" + uuid.uuid4().hex
    parts = [
        f"--{boundary}".encode(),
        b'Content-Disposition: form-data; name="access_token"\r\n',
        token.encode(),
    ]
    if title:
        parts.extend([f"--{boundary}".encode(), b'Content-Disposition: form-data; name="title"\r\n', title.encode()])
    if description:
        parts.extend([f"--{boundary}".encode(), b'Content-Disposition: form-data; name="description"\r\n', description.encode()])
    if scheduled_publish_time:
        parts.extend([
            f"--{boundary}".encode(), b'Content-Disposition: form-data; name="published"\r\n', b"false",
            f"--{boundary}".encode(), b'Content-Disposition: form-data; name="scheduled_publish_time"\r\n', str(scheduled_publish_time).encode(),
        ])
    parts.extend([
        f"--{boundary}".encode(),
        f'Content-Disposition: form-data; name="source"; filename="{file_name}"\r\nContent-Type: video/mp4\r\n'.encode(),
        video_bytes,
        f"--{boundary}--\r\n".encode(),
    ])

    body = b"\r\n".join(parts)
    upload_url = f"https://graph-video.facebook.com/v21.0/{tid}/videos"
    req = urllib.request.Request(
        upload_url,
        data=body,
        headers={
            "Content-Type": f"multipart/form-data; boundary={boundary}",
            "Content-Length": str(len(body)),
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body_err = exc.read().decode("utf-8", errors="replace")
        return {"error": {"code": exc.code, "message": str(exc), "details": body_err}}
    except Exception as exc:
        return {"error": {"code": 500, "message": str(exc)}}


def upload_and_publish_video(
    cfg: Dict[str, Any],
    video_bytes: bytes,
    file_name: str = "video.mp4",
    title: str = "New Reel",
    description: str = "",
    target_id: Optional[str] = None,
    scheduled_publish_time: Optional[int] = None,
    enable_auto_translate: Optional[bool] = None,
    translated_languages: Optional[List[str]] = None,
    publish_type: str = "REEL",
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Orchestrates video upload and publishing via Graph API or chunked GraphQL."""
    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"status": "error", "step": "validation", "details": {"error": "No target_id or actor_id configured."}}

    token = cfg.get("access_token") or (fetch_page_access_token(cfg, transport=transport) if cfg.get("cookie") else None)
    if token:
        graph_res = upload_video_graph(
            cfg, video_bytes, file_name=file_name, title=title, description=description,
            target_id=tid, page_token=token, scheduled_publish_time=scheduled_publish_time,
        )
        if "id" in graph_res:
            return {"status": "success", "video_id": graph_res["id"], "target_id": tid, "publish_result": graph_res}

    waterfall = str(uuid.uuid4())
    session = VideoUploadSession(waterfall_id=waterfall, file_size=len(video_bytes), file_name=file_name)

    init_res = init_video_upload(cfg, session.file_size, "mp4", tid, transport=transport)
    if "error" in init_res:
        return {"status": "error", "step": "init", "details": init_res}

    start_res = start_video_upload(cfg, session, tid, transport=transport)
    if "error" in start_res:
        return {"status": "error", "step": "start", "details": start_res}
    if not session.video_id:
        return {"status": "error", "step": "start", "details": {"error": "No video_id assigned by start step."}}

    if not session.skip_upload:
        offset = int(session.start_offset or 0)
        while offset < len(video_bytes):
            chunk = video_bytes[offset:offset + DEFAULT_CHUNK_SIZE]
            chunk_res = upload_video_chunk(cfg, session, chunk, offset, transport=transport)
            if "error" in chunk_res:
                return {"status": "error", "step": "chunk", "offset": offset, "details": chunk_res}
            offset += len(chunk)

    finish_res = finish_video_upload(cfg, session, tid, transport=transport)
    if "error" in finish_res:
        return {"status": "error", "step": "finish", "details": finish_res}

    auto_trans = cfg.get("auto_translate", True) if enable_auto_translate is None else bool(enable_auto_translate)
    spec = PublishSpec(
        target_id=tid,
        video_id=session.video_id,
        title=title,
        description=description,
        publish_type=publish_type,
        scheduled_publish_time=scheduled_publish_time,
        enable_auto_translate=auto_trans,
        translated_languages=translated_languages,
    )
    pub_res = publish_reel_or_post(cfg, spec, transport=transport)
    if "error" in pub_res:
        return {"status": "error", "step": "publish", "details": pub_res}

    return {"status": "success", "video_id": session.video_id, "waterfall_id": waterfall, "target_id": tid, "publish_result": pub_res}


class FacebookUploadUseCase:
    """Application use case service implementing FacebookUploadUseCasePort."""

    def __init__(self, cfg: Optional[Dict[str, Any]] = None, transport: Optional[FacebookTransportPort] = None):
        self.cfg = cfg or load_module_config()
        self.transport = transport or StandardFacebookTransport()

    def upload_and_publish(
        self,
        video_bytes: bytes,
        file_name: str = "video.mp4",
        title: str = "New Reel",
        description: str = "",
        target_id: Optional[str] = None,
        scheduled_publish_time: Optional[int] = None,
        enable_auto_translate: Optional[bool] = None,
        translated_languages: Optional[List[str]] = None,
        publish_type: str = "REEL",
    ) -> Dict[str, Any]:
        return upload_and_publish_video(
            self.cfg, video_bytes, file_name=file_name, title=title, description=description,
            target_id=target_id, scheduled_publish_time=scheduled_publish_time,
            enable_auto_translate=enable_auto_translate, translated_languages=translated_languages,
            publish_type=publish_type, transport=self.transport,
        )


def _post_form_request(
    url: str,
    form_data: Dict[str, Any],
    cfg: Dict[str, Any],
    transport: Optional[FacebookTransportPort] = None,
) -> Dict[str, Any]:
    """Internal helper to execute form-encoded POST requests."""
    headers = build_authenticated_headers(cfg, {"Content-Type": "application/x-www-form-urlencoded"})
    tp = transport or StandardFacebookTransport()
    return tp.post_form(url, form_data, headers, timeout=30)
