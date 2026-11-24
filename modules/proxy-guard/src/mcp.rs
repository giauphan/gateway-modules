//! Model Context Protocol (MCP) tool execution and schema adapter.
//!
//! Clean Architecture: Interface Adapters (MCP).
//! Exposes domain capabilities directly to AI agents via Toolcase Gateway.

use crate::domain::{
    FlowRequestDto, ProxyRotationStrategy, ProxySelectionQuery, RuleCategory, UpstreamProxy,
};
use crate::ports::{FlowOptimizerPort, MetricsPort, ProxyPoolManagementPort, RuleManagementPort};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_mcp_tool_request(
    tool_name: &str,
    arguments: &Value,
    service: &ProxyGuardService,
) -> Value {
    let clean_tool = tool_name.strip_prefix("proxy-guard__").unwrap_or(tool_name);

    match clean_tool {
        "evaluate_url" => {
            let url = match arguments.get("url").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return make_mcp_error("Missing required parameter 'url'"),
            };
            let method = arguments
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET");
            let resource_type = arguments.get("resource_type").and_then(|v| v.as_str());

            let decision = service.evaluate_url(url, method, resource_type);
            make_mcp_success(&json!({
                "decision": decision,
                "summary": if decision.is_blocked() {
                    format!("BLOCKED: {} ({:?}) - Saves ~{} bytes", decision.reason, decision.action, decision.estimated_bytes_saved)
                } else {
                    "FORWARDED to upstream paid proxy".to_string()
                }
            }))
        }
        "optimize_flow" => {
            let requests_val = match arguments.get("requests").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => return make_mcp_error("Missing required array parameter 'requests'"),
            };

            let mut req_dtos = Vec::with_capacity(requests_val.len());
            for item in requests_val {
                let url = item.get("url").and_then(|v| v.as_str()).or_else(|| {
                    item.get("request")
                        .and_then(|r| r.get("url"))
                        .and_then(|v| v.as_str())
                });

                let url = match url {
                    Some(u) => u,
                    None => {
                        return make_mcp_error(
                            "Invalid request entry: missing 'url' or nested HAR 'request.url'",
                        );
                    }
                };

                let method = item
                    .get("method")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        item.get("request")
                            .and_then(|r| r.get("method"))
                            .and_then(|v| v.as_str())
                    })
                    .map(str::to_string);

                let resource_type = item
                    .get("resource_type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                let size_bytes = item.get("size_bytes").and_then(|v| v.as_u64()).or_else(|| {
                    item.get("response")
                        .and_then(|resp| {
                            resp.get("bodySize")
                                .or_else(|| resp.get("content").and_then(|c| c.get("size")))
                        })
                        .and_then(|v| v.as_u64())
                });

                req_dtos.push(FlowRequestDto {
                    url: url.to_string(),
                    method,
                    resource_type,
                    size_bytes,
                });
            }

            let result = service.optimize_flow(&req_dtos);
            make_mcp_success(&json!({
                "flow_summary": {
                    "total_requests": result.total_flow_requests,
                    "blocked_requests": result.blocked_requests,
                    "forwarded_requests": result.forwarded_requests,
                    "original_estimated_mb": result.original_estimated_mb,
                    "optimized_estimated_mb": result.optimized_estimated_mb,
                    "bandwidth_saved_percent": format!("{}%", result.reduction_percentage),
                    "estimated_dollar_saved_usd": format!("${}", result.cost_saved_usd)
                },
                "decisions": result.decisions
            }))
        }
        "generate_script" => {
            let target = match arguments.get("target").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return make_mcp_error("Missing required parameter 'target'"),
            };

            match service.generate_client_script(target) {
                Ok(code) => make_mcp_success(&json!({
                    "target": target,
                    "interceptor_code": code
                })),
                Err(err) => make_mcp_error(&err),
            }
        }
        "get_metrics" => {
            let metrics = service.get_metrics();
            make_mcp_success(&json!({ "metrics": metrics }))
        }
        "configure_rules" => {
            if let Some(b) = arguments.get("block_analytics").and_then(|v| v.as_bool()) {
                if let Err(e) = service.toggle_category(RuleCategory::Analytics, b) {
                    return make_mcp_error(&e);
                }
            }
            if let Some(b) = arguments.get("block_ads").and_then(|v| v.as_bool()) {
                if let Err(e) = service.toggle_category(RuleCategory::Advertising, b) {
                    return make_mcp_error(&e);
                }
            }
            if let Some(b) = arguments.get("block_telemetry").and_then(|v| v.as_bool()) {
                if let Err(e) = service.toggle_category(RuleCategory::Telemetry, b) {
                    return make_mcp_error(&e);
                }
            }
            if let Some(b) = arguments.get("block_media").and_then(|v| v.as_bool()) {
                if let Err(e) = service.toggle_category(RuleCategory::HeavyMedia, b) {
                    return make_mcp_error(&e);
                }
            }
            if let Some(b) = arguments.get("block_fonts").and_then(|v| v.as_bool()) {
                if let Err(e) = service.toggle_category(RuleCategory::Font, b) {
                    return make_mcp_error(&e);
                }
            }
            if let Some(arr) = arguments
                .get("add_whitelist_domains")
                .and_then(|v| v.as_array())
            {
                for d in arr.iter().filter_map(|v| v.as_str()) {
                    if let Err(e) = service.add_whitelist_domain(d) {
                        return make_mcp_error(&e);
                    }
                }
            }
            if let Some(arr) = arguments
                .get("add_blocked_domains")
                .and_then(|v| v.as_array())
            {
                for d in arr.iter().filter_map(|v| v.as_str()) {
                    if let Err(e) = service.add_blocked_domain(d, None) {
                        return make_mcp_error(&e);
                    }
                }
            }

            if let Some(rate) = arguments.get("cost_per_gb_usd").and_then(|v| v.as_f64()) {
                if let Err(e) = service.set_cost_per_gb(rate) {
                    return make_mcp_error(&e);
                }
            }

            make_mcp_success(&json!({
                "status": "success",
                "message": "Rules configuration updated successfully"
            }))
        }
        "add_upstream_proxy" => {
            if let Some(arr) = arguments.get("proxies").and_then(|v| v.as_array()) {
                let mut added_ids = Vec::with_capacity(arr.len());
                let mut errors = Vec::new();

                for (idx, item) in arr.iter().enumerate() {
                    let proxy_res = match item {
                        Value::String(s) => UpstreamProxy::parse(s, None, None, None),
                        Value::Object(obj) => {
                            let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            let name = obj.get("name").and_then(|v| v.as_str());
                            let country = obj.get("country").and_then(|v| v.as_str());
                            let tags = obj.get("tags").and_then(|v| v.as_array()).map(|t| {
                                t.iter()
                                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                    .collect()
                            });
                            UpstreamProxy::parse(url, name, country, tags)
                        }
                        _ => Err(format!("Item at index {idx} has invalid format")),
                    };

                    match proxy_res {
                        Ok(proxy) => match service.add_proxy(proxy) {
                            Ok(id) => added_ids.push(id),
                            Err(e) => errors.push(format!("Index {idx}: {e}")),
                        },
                        Err(e) => errors.push(format!("Index {idx}: {e}")),
                    }
                }

                make_mcp_success(&json!({
                    "status": "success",
                    "added_count": added_ids.len(),
                    "added_ids": added_ids,
                    "errors": errors
                }))
            } else {
                let url = match arguments.get("url").and_then(|v| v.as_str()) {
                    Some(u) => u,
                    None => return make_mcp_error("Missing required parameter 'url'"),
                };
                let name = arguments.get("name").and_then(|v| v.as_str());
                let country = arguments.get("country").and_then(|v| v.as_str());
                let tags = arguments.get("tags").and_then(|v| v.as_array()).map(|t| {
                    t.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                });

                match UpstreamProxy::parse(url, name, country, tags) {
                    Ok(proxy) => match service.add_proxy(proxy) {
                        Ok(id) => make_mcp_success(&json!({
                            "status": "success",
                            "message": "Proxy registered in pool",
                            "id": id
                        })),
                        Err(e) => make_mcp_error(&e),
                    },
                    Err(e) => make_mcp_error(&format!("Invalid proxy URL: {e}")),
                }
            }
        }
        "list_upstream_proxies" => {
            let list: Vec<_> = service
                .list_proxies()
                .into_iter()
                .map(|p| p.to_masked())
                .collect();
            let strategy = service.get_strategy();
            make_mcp_success(&json!({
                "status": "success",
                "strategy": strategy,
                "total_proxies": list.len(),
                "proxies": list
            }))
        }
        "get_next_proxy" => {
            let query = ProxySelectionQuery {
                session_id: arguments
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                country: arguments
                    .get("country")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                tag: arguments
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                preferred_id: arguments
                    .get("preferred_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            };

            match service.select_proxy(&query) {
                Some(proxy) => {
                    let playwright_cfg = proxy.to_playwright_config();
                    make_mcp_success(&json!({
                        "status": "success",
                        "proxy": proxy,
                        "playwright_config": playwright_cfg,
                        "curl_arg": format!("-x {}", proxy.url)
                    }))
                }
                None => make_mcp_error("No available upstream proxy in pool"),
            }
        }
        "remove_upstream_proxy" => {
            let id = match arguments.get("id").and_then(|v| v.as_str()) {
                Some(i) => i,
                None => return make_mcp_error("Missing required parameter 'id'"),
            };
            match service.remove_proxy(id) {
                Ok(_) => make_mcp_success(&json!({
                    "status": "success",
                    "message": format!("Proxy '{id}' removed from pool")
                })),
                Err(e) => make_mcp_error(&e),
            }
        }
        "set_rotation_strategy" => {
            let strat_str = match arguments.get("strategy").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return make_mcp_error("Missing parameter 'strategy'"),
            };

            let strat = match strat_str.to_ascii_lowercase().as_str() {
                "round_robin" => ProxyRotationStrategy::RoundRobin,
                "random" => ProxyRotationStrategy::Random,
                "sticky_session" | "sticky" => ProxyRotationStrategy::StickySession,
                "lowest_failures" | "health" => ProxyRotationStrategy::LowestFailures,
                _ => return make_mcp_error("Invalid strategy. Options: round_robin, random, sticky_session, lowest_failures"),
            };

            match service.set_strategy(strat) {
                Ok(_) => make_mcp_success(&json!({
                    "status": "success",
                    "message": format!("Strategy set to {:?}", strat)
                })),
                Err(e) => make_mcp_error(&e),
            }
        }
        "chat" => {
            let resp = crate::handlers::chat::handle_chat_completions(arguments, service);
            if let Some(content) = resp
                .get("body")
                .and_then(|b| b.get("choices"))
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|ch| ch.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
            {
                make_mcp_success(&json!({ "reply": content }))
            } else {
                make_mcp_success(&resp["body"])
            }
        }
        "list_models" => {
            let resp = crate::handlers::chat::handle_models();
            make_mcp_success(&resp["body"])
        }
        _ => make_mcp_error(&format!("Unknown MCP tool '{tool_name}'")),
    }
}

fn make_mcp_success(data: &Value) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(data).unwrap_or_default()
            }
        ],
        "isError": false
    })
}

fn make_mcp_error(message: &str) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": json!({ "error": message }).to_string()
            }
        ],
        "isError": true
    })
}
