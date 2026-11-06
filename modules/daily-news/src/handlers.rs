//! Interface Adapters: HTTP/JSON-RPC route handlers and request dispatcher.
//!
//! Decouples external request parsing and response framing from the stdio
//! runtime loop in `main.rs`, adhering to Clean Architecture principles.

use crate::domain::RawNewsItem;
use crate::mcp::{get_mcp_tools_list, handle_mcp_tool_request};
use crate::rpc::{
    extract_date_range, extract_sources, extract_topics, make_html_resp, make_json_resp,
    parse_request_body,
};
use crate::service::DailyNewsService;
use crate::ui::render_dashboard_html;
use serde_json::{json, Value};
use std::path::Path;

pub fn handle_health(service: &DailyNewsService) -> Value {
    let cfg = service.get_config();
    make_json_resp(
        200,
        json!({
            "status": "healthy",
            "module": "daily-news",
            "version": env!("CARGO_PKG_VERSION"),
            "engine": "rust-native",
            "consumed_count": service.consumed_count(),
            "cached_candidates": service.cached_candidates_count(),
            "newsnow_url": cfg.newsnow_url,
            "redis_enabled": cfg.redis_enabled,
            "reddit_enabled": cfg.reddit_enabled,
            "reddit_subreddits": cfg.reddit_subreddits
        }),
    )
}

pub fn handle_models() -> Value {
    make_json_resp(
        200,
        json!({
            "object": "list",
            "data": [{
                "id": "daily-news",
                "object": "model",
                "created": 1793618000,
                "owned_by": "daily-news-rs"
            }]
        }),
    )
}

pub fn handle_ui(service: &DailyNewsService, limit: usize) -> Value {
    let cfg = service.get_config();
    let candidates = service.get_candidates(limit.max(10));
    let html = render_dashboard_html(
        env!("CARGO_PKG_VERSION"),
        "rust-native",
        &cfg,
        service.consumed_count(),
        &candidates,
    );
    make_html_resp(200, &html)
}

pub fn handle_flowchart() -> Value {
    make_html_resp(200, include_str!("../docs/flowchart.html"))
}

pub fn handle_chat_completions(service: &DailyNewsService, limit: usize) -> Value {
    let content = service.chat_completion(limit);
    make_json_resp(
        200,
        json!({
            "id": format!("chatcmpl-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            "object": "chat.completion",
            "model": "daily-news",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }]
        }),
    )
}

pub fn handle_discover(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
    let (from_date, to_date) = extract_date_range(body);
    let topics = extract_topics(body);
    let curated: Option<Vec<RawNewsItem>> = body["curated_candidates"]
        .as_array()
        .map(|arr| serde_json::from_value(json!(arr)).unwrap_or_default());
    let sources = extract_sources(body);

    match service.discover(
        limit,
        curated,
        sources,
        from_date.as_deref(),
        to_date.as_deref(),
        topics.as_deref(),
    ) {
        Ok(result) => make_json_resp(200, json!(result)),
        Err((status_code, err_msg)) => make_json_resp(
            400,
            json!({
                "tier": "none",
                "status": status_code,
                "candidates": [],
                "reason": err_msg
            }),
        ),
    }
}

pub fn handle_candidates(
    service: &DailyNewsService,
    method: &str,
    body: &Value,
    limit: usize,
) -> Value {
    if method == "POST" && body.get("items").is_some() {
        let raw_items: Vec<RawNewsItem> =
            serde_json::from_value(body["items"].clone()).unwrap_or_default();
        let (from_date, to_date) = extract_date_range(body);
        let from_epoch = from_date
            .as_deref()
            .and_then(|d| crate::date::parse_boundary_datetime(d, false).ok());
        let to_epoch = to_date
            .as_deref()
            .and_then(|d| crate::date::parse_boundary_datetime(d, true).ok());
        let topics = extract_topics(body);

        let ranked =
            service.filter_and_score(&raw_items, from_epoch, to_epoch, topics.as_deref(), limit);
        let count = ranked.len();
        return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
    }

    let ranked = service.get_candidates(limit);
    let count = ranked.len();
    make_json_resp(200, json!({ "count": count, "candidates": ranked }))
}

pub fn handle_feed(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
    let sources = extract_sources(body);
    let mut feed = if let Some(ref s) = sources {
        service.get_feed_sources(s, limit)
    } else {
        service.get_feed(limit)
    };
    feed.truncate(limit);
    let count = feed.len();
    make_json_resp(200, json!({ "count": count, "items": feed }))
}

pub fn handle_consume(service: &DailyNewsService, body: &Value) -> Value {
    let ids: Vec<String> = body["candidate_ids"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
                .collect()
        })
        .or_else(|| {
            body["candidate_id"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing or empty 'candidate_id' or 'candidate_ids' parameter" }),
        );
    }

    match service.consume(&ids) {
        Ok(count) => make_json_resp(200, json!({ "status": "consumed", "count": count })),
        Err(e) => make_json_resp(500, json!({ "error": e })),
    }
}

pub fn handle_discussion(service: &DailyNewsService, body: &Value) -> Value {
    let url_or_id = body["url"]
        .as_str()
        .or_else(|| body["discussion_url"].as_str())
        .or_else(|| body["id"].as_str())
        .unwrap_or("");

    if url_or_id.trim().is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'url', 'discussion_url', or 'id'" }),
        );
    }

    let limit = body["limit"].as_u64().map(|v| v as usize);
    match service.get_discussion(url_or_id, limit) {
        Ok(thread) => make_json_resp(200, json!(thread)),
        Err(e) => make_json_resp(404, json!({ "error": e })),
    }
}

pub fn handle_capture(service: &DailyNewsService, body: &Value) -> Value {
    let extract_url = |k: &str| body[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    let url = extract_url("url")
        .or_else(|| extract_url("source_url"))
        .or_else(|| extract_url("canonical_url"))
        .unwrap_or("");
    if url.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'url', 'source_url', or 'canonical_url'" }),
        );
    }
    match service.capture_primary_source(url) {
        Ok(evidence) => make_json_resp(200, json!(evidence)),
        Err(e) => make_json_resp(400, json!({ "error": e })),
    }
}

pub fn handle_enrich_candidate(
    service: &DailyNewsService,
    candidate_id: &str,
    body: &Value,
) -> Value {
    let override_url = body["override_url"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["url"].as_str().filter(|s| !s.trim().is_empty()));

    let claims = match body.get("claims").filter(|v| !v.is_null()) {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(c) => Some(c),
            Err(e) => {
                return make_json_resp(400, json!({ "error": format!("Invalid 'claims': {e}") }))
            }
        },
        None => None,
    };

    let visuals_val = body
        .get("visual_evidence")
        .filter(|v| !v.is_null())
        .or_else(|| body.get("visuals").filter(|v| !v.is_null()));

    let visuals = match visuals_val {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(vis) => Some(vis),
            Err(e) => {
                return make_json_resp(
                    400,
                    json!({ "error": format!("Invalid 'visual_evidence': {e}") }),
                )
            }
        },
        None => None,
    };

    match service.enrich_candidate(candidate_id, override_url, claims, visuals) {
        Ok(resp) => make_json_resp(200, json!(resp)),
        Err(e) => make_json_resp(400, json!({ "error": e })),
    }
}

pub fn handle_enrich(service: &DailyNewsService, body: &Value) -> Value {
    if let Some(cid_val) = body.get("candidate_id") {
        let cid = cid_val.as_str().unwrap_or("").trim();
        if cid.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'candidate_id'" }),
            );
        }
        return handle_enrich_candidate(service, cid, body);
    }
    handle_capture(service, body)
}

pub fn handle_evidence(base_dir: &Path, body: &Value) -> Value {
    let sha256_raw = body["sha256"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["hash"].as_str().filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
        .trim();

    if sha256_raw.is_empty()
        || sha256_raw.len() != 64
        || !sha256_raw.chars().all(|c| c.is_ascii_hexdigit())
    {
        return make_json_resp(
            400,
            json!({ "error": "Invalid or missing sha256 hash parameter (expected 64 hex characters)" }),
        );
    }
    let sha256 = sha256_raw.to_ascii_lowercase();

    let state_dir = crate::storage::resolve_state_dir(base_dir);
    let evidence_path = state_dir
        .join("evidence")
        .join(format!("{sha256}.txt"));
    if !evidence_path.exists() {
        return make_json_resp(
            404,
            json!({ "error": format!("Evidence snapshot not found for sha256: {sha256}") }),
        );
    }

    match std::fs::read_to_string(&evidence_path) {
        Ok(content) => make_json_resp(
            200,
            json!({
                "sha256": sha256,
                "ref": format!(".state/evidence/{sha256}.txt"),
                "content": content
            }),
        ),
        Err(e) => make_json_resp(
            500,
            json!({ "error": format!("Failed to read snapshot file: {e}") }),
        ),
    }
}

pub fn handle_handoff(service: &DailyNewsService, body: &Value) -> Value {
    let action = body["action"].as_str().unwrap_or("reserve");
    let cid = body["candidate_id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .map(str::trim)
        .unwrap_or("");
    if cid.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'candidate_id'" }),
        );
    }

    if action == "release" {
        let token = body["token"]
            .as_str()
            .or_else(|| body["reservation_token"].as_str())
            .unwrap_or("");
        if token.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'token'" }),
            );
        }
        return match service.release_handoff(cid, token) {
            Ok(true) => make_json_resp(200, json!({ "status": "RELEASED", "candidate_id": cid })),
            Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
            Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
        };
    }

    if action == "commit" {
        let token = body["token"]
            .as_str()
            .or_else(|| body["reservation_token"].as_str())
            .unwrap_or("");
        if token.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'token'" }),
            );
        }
        return match service.commit_handoff(cid, token) {
            Ok(true) => make_json_resp(200, json!({ "status": "PUBLISHED", "candidate_id": cid })),
            Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
            Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
        };
    }

    match service.verify_production_handoff(cid) {
        Ok(handoff) => make_json_resp(
            200,
            json!({
                "status": "APPROVED",
                "candidate_id": cid,
                "readiness": handoff.candidate.research.research_readiness.as_ref().map(|r| r.status.as_str()).unwrap_or("UNKNOWN"),
                "candidate": handoff.candidate,
                "reservation_token": handoff.reservation_token,
                "lease_seconds": handoff.lease_seconds,
                "expires_at": handoff.expires_at,
            }),
        ),
        Err((code, msg)) => make_json_resp(
            422,
            json!({
                "status": "REJECTED",
                "code": code,
                "error": msg
            }),
        ),
    }
}

pub fn handle_config(service: &DailyNewsService, method: &str, body: &Value) -> Value {
    if method == "POST" {
        if !body.is_object() {
            return make_json_resp(
                400,
                json!({ "error": "Config payload must be a JSON object" }),
            );
        }
        let mut cfg = service.get_config();
        if let Err(e) = cfg.apply_updates_validated(body) {
            return make_json_resp(400, json!({ "error": e }));
        }
        return match service.update_config(cfg) {
            Ok(updated) => make_json_resp(
                200,
                json!({ "status": "updated", "config": updated.to_sanitized_value() }),
            ),
            Err(e) => make_json_resp(500, json!({ "error": e })),
        };
    }
    let current = service.get_config();
    make_json_resp(200, current.to_sanitized_value())
}

fn method_not_allowed(allowed: &[&str]) -> Value {
    json!({
        "status": 405,
        "headers": {
            "content-type": "application/json",
            "allow": allowed.join(", "),
            "x-module-engine": "rust-native"
        },
        "body": serde_json::to_string(&json!({
            "error": format!("Method Not Allowed: expected {}", allowed.join(", "))
        })).unwrap_or_default()
    })
}

fn extract_header_tool(req: &Value) -> Option<&str> {
    let h = req.get("headers")?;
    if let Some(arr) = h.as_array() {
        for entry in arr {
            if let Some(pair) = entry.as_array() {
                if pair.len() == 2 && pair[0].as_str() == Some("x-mcp-tool") {
                    return pair[1].as_str();
                }
            }
        }
    } else if let Some(map) = h.as_object() {
        if let Some(v) = map.get("x-mcp-tool").and_then(|v| v.as_str()) {
            return Some(v);
        }
    }
    None
}

fn enforce_method(method: &str, allowed: &[&str]) -> Result<(), Value> {
    if allowed.iter().any(|&m| method.eq_ignore_ascii_case(m)) {
        Ok(())
    } else {
        Err(method_not_allowed(allowed))
    }
}

fn dispatch_mcp(
    route: &str,
    method: &str,
    body: &Value,
    header_tool: Option<&str>,
    mount: &str,
    service: &DailyNewsService,
    base_dir: &Path,
) -> Option<Value> {
    match route {
        "/mcp" | "/api/v1/mcp" | "/tools/call" | "/api/v1/tools/call" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_mcp_tool_request(service, base_dir, body, header_tool))
        }
        "/mcp/sse" | "/sse" | "/api/v1/mcp/sse" => {
            if method.eq_ignore_ascii_case("GET") {
                Some(json!({
                    "status": 200,
                    "headers": {
                        "content-type": "text/event-stream",
                        "cache-control": "no-cache",
                        "connection": "keep-alive",
                        "x-module-engine": "rust-native"
                    },
                    "body": format!("event: endpoint\ndata: {mount}/mcp\n\n")
                }))
            } else if method.eq_ignore_ascii_case("POST") {
                Some(handle_mcp_tool_request(service, base_dir, body, header_tool))
            } else {
                Some(method_not_allowed(&["GET", "POST"]))
            }
        }
        "/tools/list" | "/api/v1/tools/list" => {
            if let Err(err) = enforce_method(method, &["GET", "POST", "HEAD"]) {
                return Some(err);
            }
            if body.get("jsonrpc").is_some() {
                Some(handle_mcp_tool_request(service, base_dir, body, header_tool))
            } else {
                Some(make_json_resp(200, json!({ "tools": get_mcp_tools_list() })))
            }
        }
        _ => None,
    }
}

fn dispatch_read_and_discovery(
    route: &str,
    method: &str,
    body: &Value,
    limit: usize,
    service: &DailyNewsService,
) -> Option<Value> {
    match route {
        "/health" | "/api/v1/health" => {
            if let Err(err) = enforce_method(method, &["GET", "HEAD"]) {
                return Some(err);
            }
            Some(handle_health(service))
        }
        "/models" | "/api/v1/models" => {
            if let Err(err) = enforce_method(method, &["GET", "HEAD"]) {
                return Some(err);
            }
            Some(handle_models())
        }
        "" | "/ui" => {
            if let Err(err) = enforce_method(method, &["GET", "HEAD"]) {
                return Some(err);
            }
            Some(handle_ui(service, limit))
        }
        "/flow" | "/flowchart" => {
            if let Err(err) = enforce_method(method, &["GET", "HEAD"]) {
                return Some(err);
            }
            Some(handle_flowchart())
        }
        "/chat/completions" | "/v1/chat/completions" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_chat_completions(service, limit))
        }
        "/api/v1/discover" | "/discover" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_discover(service, body, limit))
        }
        "/api/v1/candidates" | "/candidates" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_candidates(service, method, body, limit))
        }
        "/api/v1/feed" | "/feed" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_feed(service, body, limit))
        }
        "/api/v1/discussion" | "/discussion" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_discussion(service, body))
        }
        _ => None,
    }
}

fn dispatch_pipeline_and_admin(
    route: &str,
    method: &str,
    body: &Value,
    service: &DailyNewsService,
    base_dir: &Path,
) -> Option<Value> {
    match route {
        "/api/v1/enrich-candidate" | "/enrich-candidate" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            let cid = body["candidate_id"].as_str().map(str::trim).unwrap_or("");
            if cid.is_empty() {
                Some(make_json_resp(
                    400,
                    json!({ "error": "Missing required parameter: 'candidate_id'" }),
                ))
            } else {
                Some(handle_enrich_candidate(service, cid, body))
            }
        }
        "/api/v1/capture" | "/capture" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_capture(service, body))
        }
        "/api/v1/enrich" | "/enrich" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_enrich(service, body))
        }
        "/api/v1/evidence" | "/evidence" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_evidence(base_dir, body))
        }
        "/api/v1/consume" | "/consume" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_consume(service, body))
        }
        "/api/v1/handoff" | "/handoff" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_handoff(service, body))
        }
        "/api/v1/handoff/release" | "/handoff/release" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            let mut b = body.clone();
            b["action"] = json!("release");
            Some(handle_handoff(service, &b))
        }
        "/api/v1/handoff/commit" | "/handoff/commit" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            let mut b = body.clone();
            b["action"] = json!("commit");
            Some(handle_handoff(service, &b))
        }
        "/api/v1/config" | "/config" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_config(service, method, body))
        }
        _ => None,
    }
}

fn dispatch_standard(
    route: &str,
    raw_path: &str,
    method: &str,
    body: &Value,
    limit: usize,
    service: &DailyNewsService,
    base_dir: &Path,
) -> Value {
    if let Some(resp) = dispatch_read_and_discovery(route, method, body, limit, service) {
        return resp;
    }
    if let Some(resp) = dispatch_pipeline_and_admin(route, method, body, service, base_dir) {
        return resp;
    }
    make_json_resp(
        404,
        json!({ "error": format!("Route not found: {raw_path}") }),
    )
}

pub fn handle_request(req: Value, service: &DailyNewsService, base_dir: &Path) -> Value {
    let body_len = match req.get("body") {
        Some(Value::String(s)) => s.len(),
        Some(v) => serde_json::to_string(v).map(|s| s.len()).unwrap_or(0),
        None => 0,
    };
    if body_len > 2 * 1024 * 1024 {
        return make_json_resp(
            413,
            json!({ "error": "Payload Too Large: request body exceeds 2MB limit" }),
        );
    }

    let raw_path = req["path"].as_str().unwrap_or("");
    let (path, query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (raw_path.trim_end_matches('/'), ""),
    };

    let method = req["method"].as_str().unwrap_or("GET").to_ascii_uppercase();

    let body = parse_request_body(&req, query);
    let limit = body["limit"]
        .as_u64()
        .or_else(|| body["count"].as_u64())
        .unwrap_or(5)
        .clamp(1, 100) as usize;

    let header_tool = extract_header_tool(&req);
    let route = path.strip_prefix("/custom/daily-news").unwrap_or(path);
    let mount = if path.starts_with("/custom/daily-news") {
        "/custom/daily-news"
    } else {
        ""
    };

    if let Some(resp) = dispatch_mcp(route, &method, &body, header_tool, mount, service, base_dir) {
        return resp;
    }

    dispatch_standard(route, raw_path, &method, &body, limit, service, base_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_evidence_uppercase_hash_normalized() {
        let tmp = std::env::temp_dir().join("test_evidence_uppercase");
        let evidence_dir = tmp.join(".state").join("evidence");
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let sample_hash = "c".repeat(64);
        let file_path = evidence_dir.join(format!("{sample_hash}.txt"));
        std::fs::write(&file_path, "sample snapshot content").unwrap();

        let uppercase_hash = "C".repeat(64);
        let body = json!({ "sha256": uppercase_hash });
        let resp = handle_evidence(&tmp, &body);
        assert_eq!(resp["status"], 200);
        let resp_body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
        assert_eq!(resp_body["sha256"], sample_hash);
        assert_eq!(resp_body["content"], "sample snapshot content");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
