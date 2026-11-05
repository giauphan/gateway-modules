//! Model Context Protocol (MCP) tool execution adapter.
//!
//! Follows Clean Architecture Interface Adapters layer:
//! Decouples incoming MCP tool calls (`/tools/call`) from application use-cases,
//! enforces strict fail-closed boundary validation, and formats standard MCP
//! Content responses for connected AI agents.

use crate::domain::evaluate_readiness_full;
use crate::rpc::{extract_sources, extract_topics, make_json_resp};
use crate::service::DailyNewsService;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolResponse {
    pub content: Vec<McpContentItem>,
    #[serde(
        default,
        rename = "isError",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_error: bool,
    #[serde(
        default,
        rename = "is_error",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub is_error_compat: bool,
}

impl McpToolResponse {
    pub fn success(text: String) -> Self {
        Self {
            content: vec![McpContentItem {
                content_type: "text".to_string(),
                text,
            }],
            is_error: false,
            is_error_compat: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![McpContentItem {
                content_type: "text".to_string(),
                text: message.into(),
            }],
            is_error: true,
            is_error_compat: true,
        }
    }
}

fn handle_discover_candidates(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let limit = (args["limit"].as_u64().unwrap_or(10) as usize).clamp(1, 100);
    let topics = extract_topics(args);
    let from_date = args["from_date"].as_str();
    let to_date = args["to_date"].as_str();
    let sources = extract_sources(args);

    match service.discover(limit, None, sources, from_date, to_date, topics.as_deref()) {
        Ok(res) => McpToolResponse::success(json!(res).to_string()),
        Err((code, msg)) => McpToolResponse::error(format!("{code}: {msg}")),
    }
}

fn handle_get_candidate(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let candidate_id = match args["candidate_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => cid,
        None => return McpToolResponse::error("Missing required parameter: 'candidate_id'"),
    };

    match service.get_candidate(candidate_id) {
        Some(c) => McpToolResponse::success(json!(c).to_string()),
        None => McpToolResponse::error(format!("Candidate '{candidate_id}' not found")),
    }
}

fn handle_enrich_candidate(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let candidate_id = match args["candidate_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => cid,
        None => return McpToolResponse::error("Missing required parameter: 'candidate_id'"),
    };

    let override_url = args["override_url"]
        .as_str()
        .filter(|s| !s.trim().is_empty());
    let claims = match args.get("claims").filter(|v| !v.is_null()) {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(c) => Some(c),
            Err(e) => return McpToolResponse::error(format!("Invalid 'claims': {e}")),
        },
        None => None,
    };
    let visuals = match args
        .get("visual_evidence")
        .or_else(|| args.get("visuals"))
        .filter(|v| !v.is_null())
    {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(vis) => Some(vis),
            Err(e) => return McpToolResponse::error(format!("Invalid 'visual_evidence': {e}")),
        },
        None => None,
    };

    match service.enrich_candidate(candidate_id, override_url, claims, visuals) {
        Ok(resp) => McpToolResponse::success(json!(resp).to_string()),
        Err(e) => McpToolResponse::error(e),
    }
}

fn handle_verify_research_readiness(
    service: &DailyNewsService,
    base_dir: &Path,
    args: &Value,
) -> McpToolResponse {
    let candidate_id = match args["candidate_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => cid,
        None => return McpToolResponse::error("Missing required parameter: 'candidate_id'"),
    };

    let candidate = match service.get_candidate(candidate_id) {
        Some(c) => c,
        None => return McpToolResponse::error(format!("Candidate '{candidate_id}' not found")),
    };

    let snapshot_text = candidate
        .research
        .primary_source
        .as_ref()
        .and_then(|ps| ps.snapshot_ref.as_deref())
        .and_then(|ref_str| {
            service
                .read_snapshot_text(ref_str)
                .or_else(|| read_snapshot_from_fs(base_dir, ref_str))
        });

    let secondary_reader = |hash: &str| -> Option<String> {
        service
            .read_snapshot_text(hash)
            .or_else(|| read_snapshot_from_fs(base_dir, hash))
    };

    let audit_reader = |hash: &str| -> Option<String> {
        service
            .read_snapshot_text(hash)
            .or_else(|| read_snapshot_from_fs(base_dir, hash))
    };

    let asset_reader = |hash: &str| -> Option<Vec<u8>> {
        read_asset_bytes_from_fs(base_dir, hash)
    };

    let readiness = evaluate_readiness_full(
        candidate.research.primary_source.as_ref(),
        snapshot_text.as_deref(),
        &candidate.research.claims,
        &candidate.research.visual_evidence,
        candidate.content_origin,
        Some(&secondary_reader),
        Some(&audit_reader),
        Some(&asset_reader),
    );

    McpToolResponse::success(json!(readiness).to_string())
}

fn handle_get_evidence_snapshot(base_dir: &Path, args: &Value) -> McpToolResponse {
    let sha256_raw = args["sha256"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| args["hash"].as_str().filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
        .trim();

    if sha256_raw.len() != 64 || !sha256_raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return McpToolResponse::error(
            "Invalid or missing sha256 parameter: expected 64 hex characters",
        );
    }
    let sha256 = sha256_raw.to_ascii_lowercase();

    match read_snapshot_from_fs(base_dir, &sha256) {
        Some(content) => McpToolResponse::success(
            json!({
                "sha256": sha256,
                "ref": format!(".state/evidence/{sha256}.txt"),
                "content": content
            })
            .to_string(),
        ),
        None => McpToolResponse::error(format!("Evidence snapshot not found for sha256: {sha256}")),
    }
}

fn handle_get_discussion(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let url_or_id = args["url_or_id"]
        .as_str()
        .or_else(|| args["url"].as_str())
        .or_else(|| args["id"].as_str())
        .unwrap_or("")
        .trim();

    if url_or_id.is_empty() {
        return McpToolResponse::error("Missing required parameter: 'url_or_id', 'url', or 'id'");
    }

    let limit = args["limit"].as_u64().map(|v| (v as usize).clamp(1, 100));
    match service.get_discussion(url_or_id, limit) {
        Ok(thread) => McpToolResponse::success(json!(thread).to_string()),
        Err(e) => McpToolResponse::error(e),
    }
}

fn handle_verify_production_handoff(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let candidate_id = match args["candidate_id"]
        .as_str()
        .or_else(|| args["id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(cid) => cid,
        None => return McpToolResponse::error("Missing required parameter: 'candidate_id'"),
    };

    match service.verify_production_handoff(candidate_id) {
        Ok(c) => McpToolResponse::success(
            json!({
                "status": "APPROVED",
                "candidate_id": candidate_id,
                "readiness": c.research.research_readiness.as_ref().map(|r| r.status.as_str()).unwrap_or("UNKNOWN"),
                "candidate": c
            })
            .to_string(),
        ),
        Err((code, msg)) => McpToolResponse::error(format!("{code}: {msg}")),
    }
}

fn read_snapshot_from_fs(base_dir: &Path, ref_or_hash: &str) -> Option<String> {
    let clean_hash = ref_or_hash
        .trim_end_matches(".txt")
        .split('/')
        .next_back()?
        .to_ascii_lowercase();
    if clean_hash.len() != 64 || !clean_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let state_dir = crate::storage::resolve_state_dir(base_dir);
    let evidence_path = state_dir
        .join("evidence")
        .join(format!("{clean_hash}.txt"));
    std::fs::read_to_string(evidence_path).ok()
}

fn read_asset_bytes_from_fs(base_dir: &Path, ref_or_hash: &str) -> Option<Vec<u8>> {
    let clean_hash = ref_or_hash
        .trim_end_matches(".bin")
        .trim_end_matches(".txt")
        .split('/')
        .next_back()?
        .to_ascii_lowercase();
    if clean_hash.len() != 64 || !clean_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let state_dir = crate::storage::resolve_state_dir(base_dir);
    let bin_path = state_dir.join("assets").join(format!("{clean_hash}.bin"));
    if let Ok(bytes) = std::fs::read(bin_path) {
        return Some(bytes);
    }
    let evidence_path = state_dir.join("evidence").join(format!("{clean_hash}.txt"));
    std::fs::read(evidence_path).ok()
}

pub fn dispatch_mcp_tool(
    service: &DailyNewsService,
    base_dir: &Path,
    tool_name: &str,
    arguments: &Value,
) -> McpToolResponse {
    let action = tool_name
        .strip_prefix("daily-news__")
        .or_else(|| tool_name.strip_prefix("daily-news_"))
        .unwrap_or(tool_name);

    match action {
        "discover_candidates" | "discover" => handle_discover_candidates(service, arguments),
        "get_candidate" | "candidate" => handle_get_candidate(service, arguments),
        "enrich_candidate" | "enrich" => handle_enrich_candidate(service, arguments),
        "verify_research_readiness" | "verify_readiness" => {
            handle_verify_research_readiness(service, base_dir, arguments)
        }
        "verify_production_handoff" | "handoff" => {
            handle_verify_production_handoff(service, arguments)
        }
        "get_evidence_snapshot" | "evidence" => handle_get_evidence_snapshot(base_dir, arguments),
        "get_discussion" | "discussion" => handle_get_discussion(service, arguments),
        other => McpToolResponse::error(format!("Unknown or unsupported MCP tool: '{other}'")),
    }
}

fn handle_single_jsonrpc_request(
    service: &DailyNewsService,
    base_dir: &Path,
    body: &Value,
) -> Option<Value> {
    if !body.is_object() {
        return Some(json!({
            "jsonrpc": "2.0",
            "id": Value::Null,
            "error": {
                "code": -32600,
                "message": "Invalid Request: expected object"
            }
        }));
    }

    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body["method"].as_str().unwrap_or("").trim();

    // Notifications are fire-and-forget per MCP spec: no response body
    if method.starts_with("notifications/") || method == "initialized" {
        return None;
    }

    const SUPPORTED_MCP_VERSIONS: &[&str] = &["2026-07-28", "2024-11-05", "2024-10-07"];

    // Validate protocolVersion in _meta if provided (stateless MCP 2026-07-28 request metadata)
    // Checks canonical "io.modelcontextprotocol/protocolVersion" key followed by legacy fallback.
    let meta_version = body
        .get("_meta")
        .and_then(|m| {
            m.get("io.modelcontextprotocol/protocolVersion")
                .or_else(|| m.get("protocolVersion"))
        })
        .or_else(|| {
            body.get("params")
                .and_then(|p| p.get("_meta"))
                .and_then(|m| {
                    m.get("io.modelcontextprotocol/protocolVersion")
                        .or_else(|| m.get("protocolVersion"))
                })
        })
        .and_then(|v| v.as_str());

    if let Some(meta_ver) = meta_version {
        if !SUPPORTED_MCP_VERSIONS.contains(&meta_ver) {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32602,
                    "message": format!("Unsupported MCP protocolVersion: '{meta_ver}'. Supported versions: 2026-07-28, 2024-11-05, 2024-10-07")
                }
            }));
        }
    }

    match method {
        "initialize" => {
            let requested_ver = body["params"]["protocolVersion"]
                .as_str()
                .unwrap_or("2024-11-05");
            let negotiated = match requested_ver {
                "2026-07-28" | "2024-11-05" | "2024-10-07" => requested_ver,
                _ => "2024-11-05",
            };
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": negotiated,
                    "capabilities": {
                        "tools": { "listChanged": false },
                        "resources": { "subscribe": false, "listChanged": false },
                        "prompts": { "listChanged": false },
                        "logging": {}
                    },
                    "serverInfo": {
                        "name": "daily-news",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }))
        }
        "server/discover" | "discovery" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resultType": "complete",
                "supportedVersions": ["2026-07-28", "2024-11-05", "2024-10-07"],
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "subscribe": false, "listChanged": false },
                    "prompts": { "listChanged": false }
                },
                "_meta": {
                    "io.modelcontextprotocol/serverInfo": {
                        "name": "daily-news",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                },
                "ttlMs": 3600000,
                "cacheScope": "public"
            }
        })),
        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        "resources/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resources": []
            }
        })),
        "resources/templates/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resourceTemplates": []
            }
        })),
        "prompts/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "prompts": []
            }
        })),
        "prompts/get" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32602,
                "message": "Prompt not found"
            }
        })),
        "logging/setLevel" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        "roots/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "roots": []
            }
        })),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": get_mcp_tools_list()
            }
        })),
        "tools/call" => {
            let params = &body["params"];
            let tool_name = params["name"]
                .as_str()
                .or_else(|| body["name"].as_str())
                .unwrap_or("")
                .trim();

            if tool_name.is_empty() {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32602,
                        "message": "Missing required parameter 'name' in tools/call params"
                    }
                }));
            }

            let arguments = params
                .get("arguments")
                .or_else(|| body.get("arguments"))
                .unwrap_or(params);

            let result = dispatch_mcp_tool(service, base_dir, tool_name, arguments);
            let meta = params.get("_meta").or_else(|| body.get("_meta"));
            let mut result_obj = json!({
                "content": result.content,
                "isError": result.is_error
            });
            if let Some(m) = meta {
                result_obj["_meta"] = m.clone();
            }
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result_obj
            }))
        }
        other => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: '{other}'")
            }
        })),
    }
}

pub fn handle_jsonrpc_mcp_request(
    service: &DailyNewsService,
    base_dir: &Path,
    body: &Value,
) -> Value {
    if let Some(batch) = body.as_array() {
        if batch.is_empty() {
            let err_res = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32600,
                    "message": "Invalid Request: empty batch"
                }
            });
            return make_json_resp(400, err_res);
        }

        let mut responses = Vec::new();
        for item in batch {
            if let Some(res) = handle_single_jsonrpc_request(service, base_dir, item) {
                responses.push(res);
            }
        }
        if responses.is_empty() {
            make_json_resp(204, Value::Null)
        } else {
            make_json_resp(200, Value::Array(responses))
        }
    } else {
        match handle_single_jsonrpc_request(service, base_dir, body) {
            Some(res) => {
                let status = if res.get("error").is_some() {
                    let code = res["error"]["code"].as_i64().unwrap_or(0);
                    if code == -32601 { 404 } else { 400 }
                } else {
                    200
                };
                make_json_resp(status, res)
            }
            None => make_json_resp(204, Value::Null),
        }
    }
}

pub fn handle_mcp_tool_request(
    service: &DailyNewsService,
    base_dir: &Path,
    body: &Value,
    header_tool: Option<&str>,
) -> Value {
    if body.is_array() || body.get("jsonrpc").is_some() {
        return handle_jsonrpc_mcp_request(service, base_dir, body);
    }

    let tool_name = header_tool
        .or_else(|| body["name"].as_str())
        .or_else(|| body["tool"].as_str())
        .or_else(|| body["params"]["name"].as_str())
        .or_else(|| {
            body["method"]
                .as_str()
                .and_then(|m| m.strip_prefix("tools/"))
        })
        .unwrap_or("")
        .trim();

    if tool_name.is_empty() {
        let resp = McpToolResponse::error("Missing tool name in MCP request payload or header");
        return make_json_resp(400, json!(resp));
    }

    let arguments = body
        .get("arguments")
        .or_else(|| body["params"].get("arguments"))
        .or_else(|| body.get("params"))
        .unwrap_or(body);

    let result = dispatch_mcp_tool(service, base_dir, tool_name, arguments);
    let status = if result.is_error { 400 } else { 200 };
    make_json_resp(status, json!(result))
}

pub use crate::mcp_schema::get_mcp_tools_list;
