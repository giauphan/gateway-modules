//! Interface Adapters: Request Router and Dispatcher.
//!
//! Clean Architecture: Interface Adapters Layer.

pub mod chat;
pub mod config;
pub mod evaluate;
pub mod proxy_pool;
pub mod rules;
pub mod stats;

pub use chat::*;
pub use config::*;
pub use evaluate::*;
pub use proxy_pool::*;
pub use rules::*;
pub use stats::*;

use crate::mcp::handle_mcp_tool_request;
use crate::rpc::{extract_request_body, make_error_resp, make_json_resp};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_request(req: Value, service: &ProxyGuardService) -> Value {
    let method = req
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let raw_path = req.get("path").and_then(|v| v.as_str()).unwrap_or("/");
    let (path_only, query_str) = match raw_path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (raw_path, ""),
    };
    let clean_path = path_only.trim_end_matches('/');

    if let Some(resp) = handle_mcp_route(&req, path_only, service) {
        return resp;
    }
    if let Some(resp) = handle_core_route(&req, clean_path, &method, service) {
        return resp;
    }
    if let Some(resp) = handle_custom_route(&req, clean_path, query_str, &method, service) {
        return resp;
    }

    make_error_resp(404, &format!("Not Found: {method} {raw_path}"))
}

fn handle_mcp_route(req: &Value, path: &str, service: &ProxyGuardService) -> Option<Value> {
    if let Some(tool_name) = extract_header_tool(req) {
        let body = extract_request_body(req).unwrap_or_else(|_| json!({}));
        let args = body.get("arguments").unwrap_or(&body);
        let mcp_res = handle_mcp_tool_request(tool_name, args, service);
        return Some(make_json_resp(200, mcp_res));
    }

    if path == "/tools/call" || path.ends_with("/tools/call") {
        let body = extract_request_body(req).unwrap_or_else(|_| json!({}));
        let tool_name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let default_args = json!({});
        let args = body.get("arguments").unwrap_or(&default_args);
        let mcp_res = handle_mcp_tool_request(tool_name, args, service);
        return Some(make_json_resp(200, mcp_res));
    }

    if path == "/tools/list" || path.ends_with("/tools/list") {
        return Some(make_json_resp(
            200,
            json!({ "tools": get_mcp_tools_list() }),
        ));
    }

    None
}

fn handle_core_route(
    req: &Value,
    clean_path: &str,
    method: &str,
    service: &ProxyGuardService,
) -> Option<Value> {
    if clean_path == "/config"
        || clean_path == "/custom/proxy-guard/config"
        || clean_path == "/modules/proxy-guard/config"
        || clean_path.ends_with("/config")
    {
        return Some(handle_config_route(req, method, service));
    }

    if clean_path == "/health"
        || clean_path == "/custom/proxy-guard/health"
        || clean_path.ends_with("/health")
    {
        return Some(make_json_resp(
            200,
            json!({
                "status": "healthy",
                "module": "proxy-guard",
                "version": env!("CARGO_PKG_VERSION")
            }),
        ));
    }

    if clean_path == "/ui" || clean_path == "/custom/proxy-guard/ui" || clean_path.ends_with("/ui")
    {
        let html = crate::ui::render_dashboard_html(service);
        return Some(crate::rpc::make_text_resp(
            200,
            "text/html; charset=utf-8",
            &html,
        ));
    }

    if clean_path == "/models" || clean_path == "/v1/models" || clean_path.ends_with("/models") {
        return Some(handle_models());
    }

    if clean_path == "/chat/completions"
        || clean_path == "/v1/chat/completions"
        || clean_path.ends_with("/chat/completions")
    {
        let body = extract_request_body(req).unwrap_or_else(|_| json!({}));
        return Some(handle_chat_completions(&body, service));
    }

    None
}

fn handle_custom_route(
    req: &Value,
    clean_path: &str,
    query_str: &str,
    method: &str,
    service: &ProxyGuardService,
) -> Option<Value> {
    if clean_path.ends_with("/evaluate") {
        let body = match extract_request_body(req) {
            Ok(b) => b,
            Err(e) => return Some(make_error_resp(400, &e)),
        };
        return Some(handle_evaluate_url(&body, service));
    }

    if clean_path.ends_with("/optimize-flow") {
        let body = match extract_request_body(req) {
            Ok(b) => b,
            Err(e) => return Some(make_error_resp(400, &e)),
        };
        return Some(handle_optimize_flow(&body, service));
    }

    if clean_path.ends_with("/rules") {
        if method == "POST" {
            let body = match extract_request_body(req) {
                Ok(b) => b,
                Err(e) => return Some(make_error_resp(400, &e)),
            };
            return Some(handle_update_rules(&body, service));
        }
        return Some(handle_get_rules(service));
    }

    if clean_path.ends_with("/metrics/reset") {
        return Some(handle_reset_metrics(service));
    }

    if clean_path.ends_with("/metrics") {
        return Some(handle_get_metrics(service));
    }

    if clean_path.ends_with("/logs/clear") {
        return Some(handle_clear_logs(service));
    }

    if clean_path.ends_with("/logs") {
        let mut body = extract_request_body(req).unwrap_or_else(|_| json!({}));
        if !query_str.is_empty() {
            let q_map = crate::rpc::parse_query_params(query_str);
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in q_map {
                    obj.entry(k).or_insert(v);
                }
            } else {
                body = Value::Object(q_map);
            }
        }
        return Some(handle_get_logs(&body, service));
    }

    handle_proxy_subroutes(req, clean_path, query_str, service)
}

fn handle_proxy_subroutes(
    req: &Value,
    clean_path: &str,
    query_str: &str,
    service: &ProxyGuardService,
) -> Option<Value> {
    if clean_path.ends_with("/proxies/list") {
        return Some(handle_list_proxies(service));
    }

    if clean_path.ends_with("/proxies/add") {
        let body = match extract_request_body(req) {
            Ok(b) => b,
            Err(e) => return Some(make_error_resp(400, &e)),
        };
        return Some(handle_add_proxy(&body, service));
    }

    if clean_path.ends_with("/proxies/remove") {
        let body = match extract_request_body(req) {
            Ok(b) => b,
            Err(e) => return Some(make_error_resp(400, &e)),
        };
        return Some(handle_remove_proxy(&body, service));
    }

    if clean_path.ends_with("/proxies/next") {
        let mut body = extract_request_body(req).unwrap_or_else(|_| json!({}));
        if !query_str.is_empty() {
            let q_map = crate::rpc::parse_query_params(query_str);
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in q_map {
                    obj.entry(k).or_insert(v);
                }
            } else {
                body = Value::Object(q_map);
            }
        }
        if let Some(q_obj) = req.get("query").and_then(|v| v.as_object()) {
            if let Some(obj) = body.as_object_mut() {
                for (k, v) in q_obj {
                    obj.entry(k.clone()).or_insert_with(|| v.clone());
                }
            } else {
                body = Value::Object(q_obj.clone());
            }
        }
        return Some(handle_get_next_proxy(&body, service));
    }

    if clean_path.ends_with("/proxies/strategy") {
        let body = match extract_request_body(req) {
            Ok(b) => b,
            Err(e) => return Some(make_error_resp(400, &e)),
        };
        return Some(handle_set_strategy(&body, service));
    }

    if let Some(target) = clean_path.strip_prefix("/custom/proxy-guard/scripts/") {
        return Some(handle_get_script(target, service));
    }

    None
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

fn get_mcp_tools_list() -> Value {
    json!([
        {
            "name": "proxy-guard__evaluate_url",
            "description": "Evaluates a target URL and determines if Proxy Guard will block, mock, or forward it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to inspect" },
                    "method": { "type": "string", "description": "HTTP method", "default": "GET" },
                    "resource_type": { "type": "string", "description": "Resource type hint" }
                },
                "required": ["url"]
            }
        },
        {
            "name": "proxy-guard__optimize_flow",
            "description": "Analyzes a batch flow, removes wasteful analytics/media, and calculates bandwidth savings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "requests": {
                        "type": "array",
                        "description": "List of HTTP request objects",
                        "items": {
                            "type": "object",
                            "properties": {
                                "url": { "type": "string" },
                                "method": { "type": "string" },
                                "resource_type": { "type": "string" },
                                "size_bytes": { "type": "number" }
                            },
                            "required": ["url"]
                        }
                    }
                },
                "required": ["requests"]
            }
        },
        {
            "name": "proxy-guard__generate_script",
            "description": "Generates browser automation interceptor scripts (Playwright, Puppeteer, mitmproxy).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "enum": ["playwright_ts", "playwright_py", "puppeteer_ts", "mitmproxy_py"],
                        "description": "Target framework"
                    }
                },
                "required": ["target"]
            }
        },
        {
            "name": "proxy-guard__get_metrics",
            "description": "Returns cumulative statistics on bandwidth and dollar savings.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "proxy-guard__get_request_logs",
            "description": "Returns live intercepted request logs and traffic reduction statistics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "number", "description": "Max number of recent logs to return (default: 50, max: 100)" }
                }
            }
        },
        {
            "name": "proxy-guard__configure_rules",
            "description": "Configures filtering options and manages whitelist/blacklist domains.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "block_analytics": { "type": "boolean" },
                    "block_ads": { "type": "boolean" },
                    "block_telemetry": { "type": "boolean" },
                    "block_media": { "type": "boolean" },
                    "block_fonts": { "type": "boolean" },
                    "cost_per_gb_usd": { "type": "number" },
                    "add_whitelist_domains": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "add_blocked_domains": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }
        },
        {
            "name": "proxy-guard__add_upstream_proxy",
            "description": "Registers one or multiple upstream proxies into the multi-proxy pool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Proxy URL (e.g. http://user:pass@host:port or socks5://host:port)" },
                    "name": { "type": "string", "description": "Optional friendly name" },
                    "country": { "type": "string", "description": "Optional ISO country code (e.g. US, VN, SG)" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "proxies": {
                        "type": "array",
                        "description": "Batch list of proxies to add at once"
                    }
                }
            }
        },
        {
            "name": "proxy-guard__list_upstream_proxies",
            "description": "Lists all registered upstream proxies in the pool with their health and failure status.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "proxy-guard__get_next_proxy",
            "description": "Selects the next upstream proxy from the pool according to rotation strategy (round-robin, sticky, country). Returns Playwright/Puppeteer ready configs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "strategy": { "type": "string", "description": "Optional rotation strategy override: round_robin, sticky_session, lowest_failures, or random" },
                    "session_id": { "type": "string", "description": "Optional session key for sticky routing" },
                    "country": { "type": "string", "description": "Optional country filter (e.g. US)" },
                    "tag": { "type": "string", "description": "Optional tag filter" },
                    "preferred_id": { "type": "string", "description": "Optional preferred proxy ID" }
                }
            }
        },
        {
            "name": "proxy-guard__remove_upstream_proxy",
            "description": "Removes an upstream proxy from the multi-proxy pool by ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Proxy ID to remove" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "proxy-guard__set_rotation_strategy",
            "description": "Configures proxy rotation strategy (round_robin, random, sticky_session, lowest_failures).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "strategy": {
                        "type": "string",
                        "enum": ["round_robin", "random", "sticky_session", "lowest_failures"],
                        "description": "Rotation strategy"
                    }
                },
                "required": ["strategy"]
            }
        },
        {
            "name": "proxy-guard__chat",
            "description": "Send a chat completion prompt, status query, or URL evaluation question to Proxy Guard.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "messages": {
                        "type": "array",
                        "description": "OpenAI-compatible messages array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "role": { "type": "string" },
                                "content": { "type": "string" }
                            },
                            "required": ["role", "content"]
                        }
                    },
                    "prompt": { "type": "string", "description": "Optional direct question or URL to evaluate" }
                }
            }
        },
        {
            "name": "proxy-guard__list_models",
            "description": "Lists OpenAI-compatible models supported by this module.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }
    ])
}
