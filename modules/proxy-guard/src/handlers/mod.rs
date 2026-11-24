//! Interface Adapters: Request Router and Dispatcher.
//!
//! Clean Architecture: Interface Adapters Layer.

pub mod evaluate;
pub mod proxy_pool;
pub mod rules;
pub mod stats;

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
    let path = req.get("path").and_then(|v| v.as_str()).unwrap_or("/");

    // 1. Check MCP tool call via header or tools/call route
    if let Some(tool_name) = extract_header_tool(&req) {
        let body = extract_request_body(&req).unwrap_or_else(|_| json!({}));
        let args = body.get("arguments").unwrap_or(&body);
        let mcp_res = handle_mcp_tool_request(tool_name, args, service);
        return make_json_resp(200, mcp_res);
    }

    if path == "/tools/call" || path.ends_with("/tools/call") {
        let body = extract_request_body(&req).unwrap_or_else(|_| json!({}));
        let tool_name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let default_args = json!({});
        let args = body.get("arguments").unwrap_or(&default_args);
        let mcp_res = handle_mcp_tool_request(tool_name, args, service);
        return make_json_resp(200, mcp_res);
    }

    if path == "/tools/list" || path.ends_with("/tools/list") {
        return make_json_resp(200, json!({ "tools": get_mcp_tools_list() }));
    }

    // 2. Health Check
    if path == "/health" || path == "/custom/proxy-guard/health" || path.ends_with("/health") {
        return make_json_resp(
            200,
            json!({
                "status": "healthy",
                "module": "proxy-guard",
                "version": env!("CARGO_PKG_VERSION")
            }),
        );
    }

    // 3. Route matching
    let clean_path = path.trim_end_matches('/');

    if clean_path.ends_with("/evaluate") {
        let body = match extract_request_body(&req) {
            Ok(b) => b,
            Err(e) => return make_error_resp(400, &e),
        };
        return handle_evaluate_url(&body, service);
    }

    if clean_path.ends_with("/optimize-flow") {
        let body = match extract_request_body(&req) {
            Ok(b) => b,
            Err(e) => return make_error_resp(400, &e),
        };
        return handle_optimize_flow(&body, service);
    }

    if clean_path.ends_with("/rules") {
        if method == "POST" {
            let body = match extract_request_body(&req) {
                Ok(b) => b,
                Err(e) => return make_error_resp(400, &e),
            };
            return handle_update_rules(&body, service);
        }
        return handle_get_rules(service);
    }

    if clean_path.ends_with("/metrics/reset") {
        return handle_reset_metrics(service);
    }

    if clean_path.ends_with("/metrics") {
        return handle_get_metrics(service);
    }

    if clean_path.ends_with("/proxies/list") {
        return handle_list_proxies(service);
    }

    if clean_path.ends_with("/proxies/add") {
        let body = match extract_request_body(&req) {
            Ok(b) => b,
            Err(e) => return make_error_resp(400, &e),
        };
        return handle_add_proxy(&body, service);
    }

    if clean_path.ends_with("/proxies/remove") {
        let body = match extract_request_body(&req) {
            Ok(b) => b,
            Err(e) => return make_error_resp(400, &e),
        };
        return handle_remove_proxy(&body, service);
    }

    if clean_path.ends_with("/proxies/next") {
        let body = extract_request_body(&req).unwrap_or_else(|_| json!({}));
        return handle_get_next_proxy(&body, service);
    }

    if clean_path.ends_with("/proxies/strategy") {
        let body = match extract_request_body(&req) {
            Ok(b) => b,
            Err(e) => return make_error_resp(400, &e),
        };
        return handle_set_strategy(&body, service);
    }

    if let Some(target) = clean_path.strip_prefix("/custom/proxy-guard/scripts/") {
        return handle_get_script(target, service);
    }

    make_error_resp(404, &format!("Not Found: {method} {path}"))
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
        }
    ])
}
