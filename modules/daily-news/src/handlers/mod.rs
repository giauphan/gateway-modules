//! Interface Adapters: HTTP/JSON-RPC route handlers and request dispatcher.
//!
//! Decomposed into cohesive Clean Architecture submodules:
//! - `admin`: health, models, UI dashboard, config, flowchart
//! - `candidates`: candidate discovery, feed aggregation, enrichment, evidence, discussion
//! - `handoff`: production readiness, reservations, handoff commit/release, publish idempotency

pub mod admin;
pub mod candidates;
pub mod handoff;

pub use admin::*;
pub use candidates::*;
pub use handoff::*;

use crate::mcp::{get_mcp_tools_list, handle_mcp_tool_request};
use crate::rpc::{make_json_resp, try_parse_request_body};
use crate::service::DailyNewsService;
use serde_json::{json, Value};
use std::path::Path;

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
        "/api/v1/publish-idempotency" | "/publish-idempotency" => {
            if let Err(err) = enforce_method(method, &["GET", "POST"]) {
                return Some(err);
            }
            Some(handle_publish_idempotency(service, method, body))
        }
        _ => None,
    }
}

fn dispatch_pipeline(
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
                return Some(make_json_resp(
                    400,
                    json!({ "error": "Missing required parameter: 'candidate_id'" }),
                ));
            }
            Some(handle_enrich_candidate(service, cid, body))
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
        "/api/v1/verify-production-handoff" | "/verify-production-handoff" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_verify_production_handoff(service, body))
        }
        "/api/v1/verify-research-readiness" | "/verify-research-readiness" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_verify_research_readiness(service, base_dir, body))
        }
        _ => None,
    }
}

fn handle_handoff_with_action(service: &DailyNewsService, body: &Value, action: &str) -> Value {
    let mut b = if body.is_object() { body.clone() } else { json!({}) };
    b["action"] = json!(action);
    handle_handoff(service, &b)
}

fn dispatch_handoff_and_admin(
    route: &str,
    method: &str,
    body: &Value,
    req: &Value,
    service: &DailyNewsService,
) -> Option<Value> {
    match route {
        "/api/v1/release-consumed" | "/release-consumed" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_release_handoff(service, body))
        }
        "/api/v1/commit-consumed" | "/commit-consumed" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_commit_handoff(service, body, req))
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
            Some(handle_handoff_with_action(service, body, "release"))
        }
        "/api/v1/handoff/commit" | "/handoff/commit" => {
            if let Err(err) = enforce_method(method, &["POST"]) {
                return Some(err);
            }
            Some(handle_handoff_with_action(service, body, "commit"))
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

fn dispatch_pipeline_and_admin(
    route: &str,
    method: &str,
    body: &Value,
    req: &Value,
    service: &DailyNewsService,
    base_dir: &Path,
) -> Option<Value> {
    dispatch_pipeline(route, method, body, service, base_dir)
        .or_else(|| dispatch_handoff_and_admin(route, method, body, req, service))
}

#[allow(clippy::too_many_arguments)]
fn dispatch_standard(
    route: &str,
    raw_path: &str,
    method: &str,
    body: &Value,
    req: &Value,
    limit: usize,
    service: &DailyNewsService,
    base_dir: &Path,
) -> Value {
    if let Some(resp) = dispatch_read_and_discovery(route, method, body, limit, service) {
        return resp;
    }
    if let Some(resp) = dispatch_pipeline_and_admin(route, method, body, req, service, base_dir) {
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

    let body = match try_parse_request_body(&req, query) {
        Ok(b) => b,
        Err(e) => {
            return make_json_resp(400, json!({ "error": e, "status": "BAD_REQUEST" }));
        }
    };

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

    dispatch_standard(route, raw_path, &method, &body, &req, limit, service, base_dir)
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
