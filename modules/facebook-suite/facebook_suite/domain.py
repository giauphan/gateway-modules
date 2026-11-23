"""
Domain entities, value objects, and pure extraction logic for Meta Business Suite.
Clean Architecture Layer: Domain Core (Zero external network/framework dependencies).
"""

from dataclasses import dataclass, field
import json
from typing import Any, Dict, List, Optional
import urllib.parse


@dataclass(frozen=True)
class FacebookCredentials:
    """Immutable value object holding Meta Business Suite session credentials."""
    user_id: str = ""
    actor_id: str = ""
    target_id: str = ""
    business_id: str = ""
    fb_dtsg: str = ""
    jazoest: str = ""
    lsd: str = ""
    cookie: str = ""
    user_agent: str = ""
    base_url: str = "https://business.facebook.com"
    enabled: bool = True
    auto_translate: bool = True

    @property
    def is_configured(self) -> bool:
        """Determines if minimum credentials for API dispatch are present."""
        return bool(self.fb_dtsg or self.cookie or self.user_id)


@dataclass
class VideoUploadSession:
    """Represents an active video chunked upload session to Meta rupload."""
    waterfall_id: str
    video_id: str = ""
    upload_session_id: str = ""
    start_offset: int = 0
    end_offset: int = 0
    file_size: int = 0
    file_name: str = "video.mp4"
    mime_type: str = "video/mp4"
    skip_upload: bool = False

    @property
    def is_complete(self) -> bool:
        return self.file_size > 0 and self.end_offset >= self.file_size


@dataclass(frozen=True)
class PublishSpec:
    """Specification for publishing a Video, Reel, or Story to Facebook."""
    target_id: str
    video_id: str
    title: str = ""
    description: str = ""
    publish_type: str = "REEL"  # REEL, STORY, or FEED
    client_mutation_id: str = ""
    scheduled_publish_time: Optional[int] = None
    enable_auto_translate: bool = True
    translated_languages: Optional[List[str]] = None


DEFAULT_TRANSLATED_LANGUAGES: List[str] = [
    "en_US", "vi_VN", "hi_IN", "ar_AR", "pt_BR",
    "es_LA", "fr_FR", "bn_IN", "id_ID", "th_TH",
    "te_IN", "ta_IN", "mr_IN", "kn_IN",
]


def build_translated_audio_metadata(
    enabled: bool = True,
    languages: Optional[List[str]] = None,
) -> List[Dict[str, Any]]:
    """Constructs translatedAudioMetadata list for Meta Business Suite video composer."""
    langs = languages if languages is not None else DEFAULT_TRANSLATED_LANGUAGES
    return [
        {
            "audio_handle": "",
            "enabled": bool(enabled),
            "is_lipsync_enabled": False,
            "lang_code": code,
            "require_review": False,
            "source_lang_code": "",
        }
        for code in langs
    ]


def derive_jazoest(fb_dtsg: str) -> str:
    """
    Computes Meta's jazoest checksum for a given fb_dtsg token.
    Algorithm: '2' + sum of ASCII ordinals of each char in fb_dtsg.
    """
    if not fb_dtsg:
        return ""
    total = sum(ord(c) for c in fb_dtsg)
    return f"2{total}"


def mask_secret(val: Optional[str], show_start: int = 4, show_end: int = 4) -> str:
    """Safely masks sensitive tokens for presentation in UI and logs."""
    if not val:
        return "not set"
    clean = str(val).strip()
    if len(clean) <= (show_start + show_end):
        return "***"
    return f"{clean[:show_start]}...{clean[-show_end:]}"


def extract_facebook_config_from_har(raw_har: Any) -> Dict[str, Any]:
    """
    Extracts Meta Business Suite configuration and credentials from a HAR capture.
    Handles raw JSON string, bytes, or parsed dictionary.
    """
    if isinstance(raw_har, bytes):
        raw_har = raw_har.decode("utf-8", errors="ignore")
    if isinstance(raw_har, str):
        try:
            data = json.loads(raw_har)
        except Exception as exc:
            raise ValueError(f"Invalid JSON payload: {exc}") from exc
    elif isinstance(raw_har, dict):
        data = raw_har
    else:
        raise ValueError("Unsupported HAR payload type")

    entries: List[Dict[str, Any]] = data.get("log", {}).get("entries", [])
    if not entries and isinstance(data.get("entries"), list):
        entries = data["entries"]

    config: Dict[str, Any] = {
        "user_id": "",
        "actor_id": "",
        "target_id": "",
        "business_id": "",
        "fb_dtsg": "",
        "jazoest": "",
        "lsd": "",
        "cookie": "",
        "user_agent": "",
    }

    for entry in entries:
        req = entry.get("request", {})
        url = req.get("url", "")
        parsed_url = urllib.parse.urlparse(url)
        hostname = (parsed_url.hostname or "").lower()
        if not (hostname == "facebook.com" or hostname.endswith(".facebook.com")):
            continue

        headers = req.get("headers", [])

        # 1. Extract from Request Headers
        for h in headers:
            name = str(h.get("name", "")).lower()
            val = str(h.get("value", ""))
            if name == "cookie" and val and not config["cookie"]:
                config["cookie"] = val
            elif name == "user-agent" and val and not config["user_agent"]:
                config["user_agent"] = val
            elif name == "x-fb-lsd" and val and not config["lsd"]:
                config["lsd"] = val

        # 2. Extract from URL Query Parameters
        q = urllib.parse.parse_qs(parsed_url.query)
        _extract_keys_from_dict(q, config)

        # 3. Extract from Form-encoded or JSON POST Data
        post_data = req.get("postData", {})
        text = post_data.get("text", "")
        if text:
            # Try form-urlencoded first
            try:
                p = urllib.parse.parse_qs(text)
                _extract_keys_from_dict(p, config)
            except Exception:
                pass

            # Try JSON body if query parsing didn't find fb_dtsg
            if not config["fb_dtsg"] and text.startswith("{"):
                try:
                    obj = json.loads(text)
                    if isinstance(obj, dict):
                        _extract_keys_from_dict(obj, config)
                except Exception:
                    pass

    # Ensure jazoest is derived if fb_dtsg was found but jazoest was not
    if config["fb_dtsg"] and not config["jazoest"]:
        config["jazoest"] = derive_jazoest(config["fb_dtsg"])

    return config


def _extract_keys_from_dict(source: Dict[str, Any], target: Dict[str, Any]) -> None:
    """Helper to populate target credentials from parsed dictionary."""
    def get_val(key: str) -> Optional[str]:
        val = source.get(key)
        if isinstance(val, list) and val:
            return str(val[0])
        if isinstance(val, (str, int)):
            return str(val)
        return None

    user = get_val("__user") or get_val("c_user")
    if user and not target["user_id"]:
        target["user_id"] = user

    actor = get_val("av") or get_val("actor_id")
    if actor and not target["actor_id"]:
        target["actor_id"] = actor

    target_id = get_val("target_id")
    if target_id and not target["target_id"]:
        target["target_id"] = target_id

    bid = get_val("__bid") or get_val("business_id")
    if bid and not target["business_id"]:
        target["business_id"] = bid

    dtsg = get_val("fb_dtsg") or get_val("fb_dtsg_ag")
    if dtsg and not target["fb_dtsg"]:
        target["fb_dtsg"] = dtsg

    jaz = get_val("jazoest")
    if jaz and not target["jazoest"]:
        target["jazoest"] = jaz

    lsd = get_val("lsd")
    if lsd and not target["lsd"]:
        target["lsd"] = lsd


def build_rupload_headers(
    session: VideoUploadSession,
    chunk_length: int,
    offset: int,
    user_agent: Optional[str] = None,
) -> Dict[str, str]:
    """Constructs compliant HTTP headers for Meta rupload video chunk uploads."""
    headers = {
        "Offset": str(offset),
        "X-Entity-Length": str(session.file_size),
        "X-Entity-Name": session.file_name,
        "X-Entity-Type": session.mime_type,
        "Content-Length": str(chunk_length),
        "Content-Type": "application/octet-stream",
        "composer_session_id": session.waterfall_id,
        "start_offset": str(offset),
        "end_offset": str(offset + chunk_length),
    }
    if session.upload_session_id:
        headers["id"] = session.upload_session_id
    if session.video_id:
        headers["product_media_id"] = session.video_id
    if user_agent:
        headers["User-Agent"] = user_agent
    return headers
