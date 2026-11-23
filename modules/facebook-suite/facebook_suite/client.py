"""
Application services and Meta Business Suite HTTP client orchestration.
Clean Architecture Layer: Application Services & Transport Adapters.
"""

import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Dict, Optional
import urllib.error
import urllib.parse
import urllib.request

from .domain import derive_jazoest, mask_secret


CONFIG_FILE_NAME = "config.json"
_RUNTIME_CONFIG_CACHE: Optional[Dict[str, Any]] = None


def get_config_path() -> Path:
    """Resolves config.json path within module directory."""
    module_dir = Path(__file__).resolve().parent.parent
    return module_dir / CONFIG_FILE_NAME


def load_module_config() -> Dict[str, Any]:
    """Loads module configuration from config.json with safe fallbacks."""
    global _RUNTIME_CONFIG_CACHE
    if _RUNTIME_CONFIG_CACHE is not None:
        return dict(_RUNTIME_CONFIG_CACHE)

    default_cfg: Dict[str, Any] = {
        "base_url": "https://business.facebook.com",
        "enabled": True,
        "user_id": "",
        "actor_id": "",
        "target_id": "",
        "business_id": "",
        "fb_dtsg": "",
        "jazoest": "",
        "lsd": "",
        "cookie": "",
        "access_token": "",
        "auto_translate": True,
        "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    }

    path = get_config_path()
    if path.is_file():
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
                if isinstance(data, dict):
                    default_cfg.update(data)
        except Exception:
            pass

    if not default_cfg.get("access_token"):
        env_tok = os.environ.get("TOKEN_FACEBOOK") or os.environ.get("FB_ACCESS_TOKEN") or os.environ.get("FACEBOOK_ACCESS_TOKEN")
        if env_tok:
            default_cfg["access_token"] = env_tok.strip()

    _RUNTIME_CONFIG_CACHE = dict(default_cfg)
    return default_cfg


def _validate_base_url(url: str) -> str:
    """Validates that base_url is an https URL pointing to facebook.com or a subdomain."""
    parsed = urllib.parse.urlparse(url)
    host = (parsed.hostname or "").lower()
    if parsed.scheme != "https" or not (host == "facebook.com" or host.endswith(".facebook.com")):
        raise ValueError("base_url must be an https facebook.com URL")
    return url.rstrip("/")


def save_module_config(updates: Dict[str, Any]) -> Dict[str, Any]:
    """Updates and persists module configuration to config.json with in-memory caching."""
    global _RUNTIME_CONFIG_CACHE
    current = load_module_config()
    allowed_keys = {
        "base_url", "enabled", "user_id", "actor_id", "target_id",
        "business_id", "fb_dtsg", "jazoest", "lsd", "cookie", "access_token", "user_agent",
        "auto_translate",
    }

    for k, v in updates.items():
        if k in allowed_keys:
            if k in ("enabled", "auto_translate"):
                if isinstance(v, bool):
                    current[k] = v
                elif v is not None:
                    raise ValueError(f"Invalid type for '{k}': expected boolean")
            elif isinstance(v, str):
                v_clean = v.strip()
                if v_clean:
                    if k == "base_url":
                        current[k] = _validate_base_url(v_clean)
                    else:
                        current[k] = v_clean
            elif v is not None:
                raise ValueError(f"Invalid type for '{k}': expected string")

    dtsg_changed = isinstance(updates.get("fb_dtsg"), str) and updates["fb_dtsg"].strip()
    jazoest_supplied = isinstance(updates.get("jazoest"), str) and updates["jazoest"].strip()
    if current.get("fb_dtsg") and (not current.get("jazoest") or (dtsg_changed and not jazoest_supplied)):
        current["jazoest"] = derive_jazoest(current["fb_dtsg"])

    _RUNTIME_CONFIG_CACHE = dict(current)

    path = get_config_path()
    try:
        tmp_path = path.with_suffix(".tmp")
        fd = os.open(tmp_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(current, f, indent=2)
        os.replace(tmp_path, path)
    except OSError as exc:
        # Gracefully tolerate read-only mounts in secure sandbox environments, but make failure visible
        sys.stderr.write(f"facebook-suite: config not persisted to disk: {exc}\n")

    return current


def build_masked_config_view(cfg: Dict[str, Any]) -> Dict[str, Any]:
    """Produces sanitized dictionary safe for public UI and status endpoints."""
    user_id = cfg.get("user_id") or ""
    actor_id = cfg.get("actor_id") or ""
    target_id = cfg.get("target_id") or ""
    business_id = cfg.get("business_id") or ""
    fb_dtsg = cfg.get("fb_dtsg") or ""
    cookie = cfg.get("cookie") or ""

    configured = bool(fb_dtsg or cookie or user_id)

    return {
        "base_url": cfg.get("base_url") or "https://business.facebook.com",
        "enabled": cfg.get("enabled", True),
        "configured": configured,
        "user_id": mask_secret(user_id, 4, 3) if user_id else "not set",
        "actor_id": mask_secret(actor_id, 4, 3) if actor_id else "not set",
        "target_id": mask_secret(target_id, 4, 3) if target_id else "not set",
        "business_id": mask_secret(business_id, 4, 3) if business_id else "not set",
        "masked_fb_dtsg": mask_secret(fb_dtsg, 6, 6) if fb_dtsg else "not set",
        "masked_cookie": mask_secret(cookie, 6, 6) if cookie else "not set",
        "has_lsd": bool(cfg.get("lsd")),
        "has_jazoest": bool(cfg.get("jazoest")),
        "user_agent": cfg.get("user_agent", ""),
    }


def build_authenticated_headers(cfg: Dict[str, Any], extra_headers: Optional[Dict[str, str]] = None) -> Dict[str, str]:
    """Constructs authenticated HTTP headers for Meta Business Suite."""
    ua = cfg.get("user_agent") or "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"
    headers = {
        "User-Agent": ua,
        "Accept": "*/*",
        "Origin": "https://business.facebook.com",
        "Referer": "https://business.facebook.com/",
        "Sec-Fetch-Site": "same-site",
        "Sec-Fetch-Mode": "cors",
        "Sec-Fetch-Dest": "empty",
    }
    cookie_val = (cfg.get("cookie") or "").strip()
    if cookie_val:
        uid = cfg.get("user_id")
        if "c_user=" not in cookie_val and "xs=" not in cookie_val:
            headers["Cookie"] = f"c_user={uid}; xs={cookie_val};" if uid else cookie_val
        elif "c_user=" not in cookie_val and "xs=" in cookie_val:
            headers["Cookie"] = f"c_user={uid}; {cookie_val}" if uid else cookie_val
        else:
            headers["Cookie"] = cookie_val
    if cfg.get("lsd"):
        headers["x-fb-lsd"] = cfg["lsd"]

    if extra_headers:
        headers.update(extra_headers)

    return headers


class FileConfigStorage:
    """Concrete implementation of ConfigStoragePort."""

    def load_config(self) -> Dict[str, Any]:
        return load_module_config()

    def save_config(self, updates: Dict[str, Any]) -> Dict[str, Any]:
        return save_module_config(updates)


class StandardFacebookTransport:
    """Concrete implementation of FacebookTransportPort using standard urllib."""

    def post_form(
        self,
        url: str,
        form_data: Dict[str, Any],
        headers: Dict[str, str],
        timeout: int = 30,
    ) -> Dict[str, Any]:
        encoded = urllib.parse.urlencode(form_data).encode("utf-8")
        req = urllib.request.Request(url, data=encoded, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                raw_text = resp.read().decode("utf-8", errors="replace")
                if raw_text.startswith("for (;;);"):
                    raw_text = raw_text[9:]
                snippet = raw_text[:200].lower()
                if raw_text.startswith("<!DOCTYPE") or "<html" in snippet or "login" in snippet:
                    return {"error": {"code": 401, "message": "Meta Business Suite session cookie expired or redirected to login.", "details": raw_text[:300]}}
                return json.loads(raw_text)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            return {
                "error": {
                    "code": exc.code,
                    "message": f"HTTP {exc.code}: {exc.reason}",
                    "details": body[:500],
                }
            }
        except Exception as exc:
            return {"error": {"code": 500, "message": str(exc)}}

    def post_binary(
        self,
        url: str,
        data: bytes,
        headers: Dict[str, str],
        timeout: int = 60,
    ) -> Dict[str, Any]:
        req = urllib.request.Request(url, data=data, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                resp.read()
                return {"status": 200, "bytes_written": len(data)}
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="replace")
            return {
                "error": {
                    "code": exc.code,
                    "message": str(exc),
                    "details": body[:400],
                }
            }
        except Exception as exc:
            return {"error": {"code": 500, "message": str(exc)}}


def execute_graphql(
    cfg: Dict[str, Any],
    friendly_name: str,
    doc_id: str,
    variables: Dict[str, Any],
    extra_form: Optional[Dict[str, Any]] = None,
    transport: Optional[StandardFacebookTransport] = None,
) -> Dict[str, Any]:
    """
    Executes a POST request against business.facebook.com/api/graphql/.
    """
    base_url = cfg.get("base_url", "https://business.facebook.com").rstrip("/")
    url = f"{base_url}/api/graphql/"

    user_id = cfg.get("user_id") or "0"
    actor_id = cfg.get("actor_id") or user_id
    dtsg = cfg.get("fb_dtsg") or ""
    jazoest = cfg.get("jazoest") or derive_jazoest(dtsg)

    form_data = {
        "av": actor_id,
        "__user": user_id,
        "__a": "1",
        "__req": "1",
        "fb_dtsg": dtsg,
        "jazoest": jazoest,
        "fb_api_req_friendly_name": friendly_name,
        "variables": json.dumps(variables),
        "doc_id": doc_id,
    }

    if cfg.get("business_id"):
        form_data["__bid"] = cfg["business_id"]
    if cfg.get("lsd"):
        form_data["lsd"] = cfg["lsd"]
    if extra_form:
        form_data.update(extra_form)

    headers = build_authenticated_headers(cfg, {"Content-Type": "application/x-www-form-urlencoded"})
    tp = transport or StandardFacebookTransport()
    res = tp.post_form(url, form_data, headers, timeout=30)
    if isinstance(res, dict) and "error" not in res and res.get("errors"):
        errs = res["errors"]
        first = errs[0] if isinstance(errs, list) and errs else {}
        message = first.get("message") if isinstance(first, dict) else None
        return {"error": {"code": 502, "message": message or "GraphQL request returned errors", "details": errs}}
    return res


def fetch_profile_or_scopes(
    cfg: Dict[str, Any],
    headers: Optional[Dict[str, str]] = None,
    transport: Optional[StandardFacebookTransport] = None,
) -> Dict[str, Any]:
    """
    Fetches the authenticated Page and User scoping info from Meta Business Suite.
    """
    actor_id = cfg.get("actor_id") or cfg.get("user_id")
    if not actor_id:
        return {"error": {"code": 400, "message": "No actor_id or user_id configured."}}
    target_id = cfg.get("target_id") or actor_id

    # If configured with fb_dtsg, attempt live GraphQL scoping query
    if cfg.get("fb_dtsg"):
        doc_id = "28183223691347905"
        res = execute_graphql(
            cfg,
            "NorthStarBusinessUnifiedScopingSelectorPopoverContainerAllFirstLevelScopesQuery",
            doc_id,
            {},
            transport=transport,
        )
        if "error" in res:
            return res
        if "data" in res:
            return res

    return {
        "status": "ok",
        "user_id": cfg.get("user_id", ""),
        "actor_id": actor_id,
        "default_target_id": target_id,
        "business_id": cfg.get("business_id", ""),
        "scopes": [
            {
                "id": target_id,
                "name": "Configured Meta Business Suite Target",
                "type": "PAGE",
                "capabilities": ["video_upload", "reels_publish", "story_publish"],
            }
        ],
    }


def fetch_page_access_token(
    cfg: Dict[str, Any],
    transport: Optional[Any] = None,
) -> Optional[str]:
    """Discovers the live Page Access Token from Meta Business Suite composer HTML."""
    if cfg.get("access_token"):
        return cfg["access_token"]

    actor_id = cfg.get("actor_id") or cfg.get("target_id")
    business_id = cfg.get("business_id", "")
    if not actor_id or not cfg.get("cookie"):
        return None

    page_url = f"https://business.facebook.com/latest/reels_composer/?asset_id={actor_id}&business_id={business_id}"
    headers = build_authenticated_headers(cfg, {"Accept": "text/html,*/*"})
    req = urllib.request.Request(page_url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            html = resp.read().decode("utf-8", errors="replace")
    except Exception:
        return None

    candidates = set(re.findall(r"EAA[A-Za-z0-9]+", html))
    for token in candidates:
        if len(token) > 50:
            url = f"https://graph.facebook.com/v21.0/me?access_token={token}"
            try:
                with urllib.request.urlopen(url, timeout=5) as r:
                    data = json.loads(r.read().decode("utf-8"))
                    if str(data.get("id")) == str(actor_id):
                        return token
            except Exception:
                continue
    return None


def publish_feed_status(
    cfg: Dict[str, Any],
    message: str,
    target_id: Optional[str] = None,
    page_token: Optional[str] = None,
    scheduled_publish_time: Optional[int] = None,
    transport: Optional[Any] = None,
) -> Dict[str, Any]:
    """Publishes a status message directly to Facebook Page Feed via Graph API."""
    tid = target_id or cfg.get("target_id") or cfg.get("actor_id")
    if not tid:
        return {"error": {"code": 400, "message": "No target_id or actor_id configured."}}

    token = page_token or cfg.get("access_token") or fetch_page_access_token(cfg, transport=transport)
    if not token:
        return {"error": {"code": 401, "message": "Could not acquire valid Page Access Token."}}

    feed_url = f"https://graph.facebook.com/v21.0/{tid}/feed"
    params = {"message": message, "access_token": token}
    if scheduled_publish_time:
        params["published"] = "false"
        params["scheduled_publish_time"] = str(scheduled_publish_time)
    payload = urllib.parse.urlencode(params).encode("utf-8")
    req = urllib.request.Request(feed_url, data=payload)
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        return {"error": {"code": exc.code, "message": str(exc), "details": body}}
    except Exception as exc:
        return {"error": {"code": 500, "message": str(exc)}}


def delete_object(
    cfg: Dict[str, Any],
    object_id: str,
    page_token: Optional[str] = None,
    transport: Optional[Any] = None,
) -> Dict[str, Any]:
    """Deletes a post or video from Facebook via Graph API."""
    if not object_id:
        return {"error": {"code": 400, "message": "No object_id specified for deletion."}}

    token = page_token or cfg.get("access_token") or fetch_page_access_token(cfg, transport=transport)
    if not token:
        return {"error": {"code": 401, "message": "Could not acquire valid Page Access Token."}}

    del_url = f"https://graph.facebook.com/v21.0/{object_id}?access_token={token}"
    req = urllib.request.Request(del_url, method="DELETE")
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        return {"error": {"code": exc.code, "message": str(exc), "details": body}}
    except Exception as exc:
        return {"error": {"code": 500, "message": str(exc)}}
