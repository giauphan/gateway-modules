//! Model Context Protocol (MCP) tool execution and schema adapter.
//!
//! Clean Architecture: Interface Adapters (MCP).
//! Exposes domain capabilities directly to AI agents via Toolcase Gateway.

mod proxy_tools;

use crate::domain::{DefaultRoute, FlowRequestDto, RuleCategory};
use crate::http_cache::HttpCacheConfig;
use crate::ports::{FlowOptimizerPort, MetricsPort, RuleManagementPort};
use crate::service::ProxyGuardService;
use proxy_tools::{
    tool_add_upstream_proxy, tool_chat, tool_get_next_proxy, tool_list_upstream_proxies,
    tool_remove_upstream_proxy, tool_set_rotation_strategy,
};
use serde_json::{json, Value};

pub fn handle_mcp_tool_request(
    tool_name: &str,
    arguments: &Value,
    service: &ProxyGuardService,
) -> Value {
    let clean_tool = tool_name.strip_prefix("proxy-guard__").unwrap_or(tool_name);

    match clean_tool {
        "evaluate_url" => tool_evaluate_url(arguments, service),
        "optimize_flow" => tool_optimize_flow(arguments, service),
        "generate_script" => tool_generate_script(arguments, service),
        "get_metrics" => make_mcp_success(&json!({ "metrics": service.get_metrics() })),
        "get_request_logs" => tool_get_request_logs(arguments, service),
        "configure_rules" => tool_configure_rules(arguments, service),
        "add_upstream_proxy" => tool_add_upstream_proxy(arguments, service),
        "list_upstream_proxies" => tool_list_upstream_proxies(service),
        "get_next_proxy" => tool_get_next_proxy(arguments, service),
        "remove_upstream_proxy" => tool_remove_upstream_proxy(arguments, service),
        "set_rotation_strategy" => tool_set_rotation_strategy(arguments, service),
        "chat" => tool_chat(arguments, service),
        "list_models" => {
            let resp = crate::handlers::chat::handle_models();
            make_mcp_success(&resp["body"])
        }
        _ => make_mcp_error(&format!("Unknown MCP tool '{tool_name}'")),
    }
}

fn tool_evaluate_url(arguments: &Value, service: &ProxyGuardService) -> Value {
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
    let summary = if decision.is_blocked() {
        format!(
            "BLOCKED: {} ({:?}) - Saves ~{} bytes",
            decision.reason, decision.action, decision.estimated_bytes_saved
        )
    } else {
        "FORWARDED to upstream paid proxy".to_string()
    };
    make_mcp_success(&json!({
        "decision": decision,
        "summary": summary
    }))
}

fn extract_flow_request(item: &Value) -> Result<FlowRequestDto, &'static str> {
    let url = item
        .get("url")
        .and_then(|v| v.as_str())
        .or_else(|| {
            item.get("request")
                .and_then(|r| r.get("url"))
                .and_then(|v| v.as_str())
        })
        .ok_or("Invalid request entry: missing 'url' or nested HAR 'request.url'")?;

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

    Ok(FlowRequestDto {
        url: url.to_string(),
        method,
        resource_type,
        size_bytes,
    })
}

fn tool_optimize_flow(arguments: &Value, service: &ProxyGuardService) -> Value {
    let requests_val = match arguments.get("requests").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return make_mcp_error("Missing required array parameter 'requests'"),
    };

    let mut req_dtos = Vec::with_capacity(requests_val.len());
    for item in requests_val {
        match extract_flow_request(item) {
            Ok(dto) => req_dtos.push(dto),
            Err(e) => return make_mcp_error(e),
        }
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

fn tool_generate_script(arguments: &Value, service: &ProxyGuardService) -> Value {
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

fn tool_get_request_logs(arguments: &Value, service: &ProxyGuardService) -> Value {
    let limit = arguments
        .get("limit")
        .and_then(|v| {
            if let Some(n) = v.as_u64() {
                Some(n as usize)
            } else if let Some(s) = v.as_str() {
                s.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(50);

    let logs = service.get_request_logs(limit);
    let metrics = service.get_metrics();
    let total = metrics.total_requests;
    let blocked = metrics.blocked_requests;
    let reduction_pct = if total > 0 {
        (blocked as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    make_mcp_success(&json!({
        "status": "success",
        "request_reduction_percentage": (reduction_pct * 10.0).round() / 10.0,
        "total_requests": total,
        "blocked_requests": blocked,
        "forwarded_requests": metrics.forwarded_requests,
        "bytes_saved": metrics.bytes_saved,
        "estimated_cost_saved_usd": metrics.estimated_cost_saved_usd,
        "logs_returned": logs.len(),
        "logs": logs
    }))
}

fn tool_configure_rules(arguments: &Value, service: &ProxyGuardService) -> Value {
    let toggles = [
        ("block_analytics", RuleCategory::Analytics),
        ("block_ads", RuleCategory::Advertising),
        ("block_telemetry", RuleCategory::Telemetry),
        ("block_media", RuleCategory::HeavyMedia),
        ("block_fonts", RuleCategory::Font),
    ];
    for (key, cat) in toggles {
        if let Some(b) = arguments.get(key).and_then(|v| v.as_bool()) {
            if let Err(e) = service.toggle_category(cat, b) {
                return make_mcp_error(&e);
            }
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
    if let Some(arr) = arguments
        .get("add_direct_bypass_domains")
        .and_then(|v| v.as_array())
    {
        for d in arr.iter().filter_map(|v| v.as_str()) {
            if let Err(e) = service.add_direct_bypass_domain(d, None) {
                return make_mcp_error(&e);
            }
        }
    }
    if let Some(arr) = arguments.get("add_pool_routes").and_then(|v| v.as_array()) {
        for item in arr {
            let domain = item.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            let pool = item.get("pool").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str());
            if !domain.is_empty() && !pool.is_empty() {
                if let Err(e) = service.add_pool_route_domain(domain, pool, name) {
                    return make_mcp_error(&e);
                }
            }
        }
    }
    if let Some(dr_val) = arguments.get("default_route") {
        if let Some(route) = DefaultRoute::from_json_value(dr_val) {
            if let Err(e) = service.set_default_route(route) {
                return make_mcp_error(&e);
            }
        }
    }
    if let Some(cache_val) = arguments.get("cache") {
        if let Ok(cache_cfg) = serde_json::from_value::<HttpCacheConfig>(cache_val.clone()) {
            service.configure_cache(cache_cfg);
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

pub(super) fn make_mcp_success(data: &Value) -> Value {
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

pub(super) fn make_mcp_error(message: &str) -> Value {
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
