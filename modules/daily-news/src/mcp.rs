//! Model Context Protocol (MCP) tool execution adapter.
//!
//! Follows Clean Architecture Interface Adapters layer:
//! Decouples incoming MCP tool calls (`/tools/call`) from application use-cases,
//! enforces strict fail-closed boundary validation, and formats standard MCP
//! Content responses for connected AI agents.

use crate::domain::evaluate_readiness_with_snapshot_store;
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

impl McpToolResponse {
    pub fn success(text: String) -> Self {
        Self {
            content: vec![McpContentItem {
                content_type: "text".to_string(),
                text,
            }],
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![McpContentItem {
                content_type: "text".to_string(),
                text: message.into(),
            }],
            is_error: true,
        }
    }
}

fn handle_discover_candidates(service: &DailyNewsService, args: &Value) -> McpToolResponse {
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
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

    let readiness = evaluate_readiness_with_snapshot_store(
        candidate.research.primary_source.as_ref(),
        snapshot_text.as_deref(),
        &candidate.research.claims,
        &candidate.research.visual_evidence,
        candidate.content_origin,
        Some(&secondary_reader),
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

    let limit = args["limit"].as_u64().map(|v| v as usize);
    match service.get_discussion(url_or_id, limit) {
        Ok(thread) => McpToolResponse::success(json!(thread).to_string()),
        Err(e) => McpToolResponse::error(e),
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
    let evidence_path = base_dir
        .join(".state")
        .join("evidence")
        .join(format!("{clean_hash}.txt"));
    std::fs::read_to_string(evidence_path).ok()
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
        "get_evidence_snapshot" | "evidence" => handle_get_evidence_snapshot(base_dir, arguments),
        "get_discussion" | "discussion" => handle_get_discussion(service, arguments),
        other => McpToolResponse::error(format!("Unknown or unsupported MCP tool: '{other}'")),
    }
}

pub fn handle_mcp_tool_request(
    service: &DailyNewsService,
    base_dir: &Path,
    body: &Value,
    header_tool: Option<&str>,
) -> Value {
    let tool_name = header_tool
        .or_else(|| body["name"].as_str())
        .or_else(|| body["tool"].as_str())
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
        .or_else(|| body.get("params"))
        .unwrap_or(body);

    let result = dispatch_mcp_tool(service, base_dir, tool_name, arguments);
    let status = if result.is_error { 400 } else { 200 };
    make_json_resp(status, json!(result))
}

pub fn get_mcp_tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "discover_candidates",
            "description": "Discover and score fresh tech and AI news candidates filtered by topic, date window, and quality thresholds.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topics": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of topics (e.g. ['rust', 'llm', 'security'])"
                    },
                    "from_date": {
                        "type": "string",
                        "description": "ISO 8601 start date timestamp (e.g. '2026-11-01T00:00:00Z')"
                    },
                    "to_date": {
                        "type": "string",
                        "description": "ISO 8601 end date timestamp (e.g. '2026-11-04T00:00:00Z')"
                    },
                    "sources": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional feed sources (e.g. ['hackernews', 'reddit'])"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Maximum number of candidate articles to return"
                    }
                }
            }
        }),
        json!({
            "name": "get_candidate",
            "description": "Retrieve full candidate details, research evidence fields, and publication metadata by candidate ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Unique canonical candidate identifier (e.g. 'cand_0123456789abcdef')"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "enrich_candidate",
            "description": "Enrich a candidate article with captured primary source snapshot, structured claims, and visual evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Candidate ID to enrich"
                    },
                    "override_url": {
                        "type": "string",
                        "description": "Optional explicit primary source URL if different from discovery canonical URL"
                    },
                    "claims": {
                        "type": "array",
                        "description": "List of grounded factual claims extracted from the article"
                    },
                    "visual_evidence": {
                        "type": "array",
                        "description": "List of validated visual assets (charts, diagrams, code blocks)"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "verify_research_readiness",
            "description": "Score video production readiness (VIDEO_READY, NEEDS_RESEARCH, NEEDS_VERIFICATION, NEEDS_VISUAL_EVIDENCE) with strict evidence grounding.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Candidate ID to evaluate for video production readiness"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "get_evidence_snapshot",
            "description": "Fetch the immutable, SHA-256 content-addressed text snapshot of a captured primary or secondary source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sha256": {
                        "type": "string",
                        "pattern": "^[a-fA-F0-9]{64}$",
                        "description": "64-character hex SHA-256 hash of the snapshot artifact"
                    }
                },
                "required": ["sha256"]
            }
        }),
        json!({
            "name": "get_discussion",
            "description": "Retrieve community reactions, commentary trees, and sentiment for a news story (explicitly marked as community opinion, not factual support).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url_or_id": {
                        "type": "string",
                        "description": "Discussion URL or post ID (e.g. Hacker News story ID or Reddit permalink)"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20,
                        "description": "Maximum number of comments to retrieve"
                    }
                },
                "required": ["url_or_id"]
            }
        }),
    ]
}
