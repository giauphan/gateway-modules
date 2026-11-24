//! Model Context Protocol (MCP) tool execution and schema adapter.
//!
//! Clean Architecture: Interface Adapters (MCP).
//! Exposes domain capabilities directly to AI agents via Toolcase Gateway.

use crate::domain::{FlowRequestDto, RuleCategory};
use crate::ports::{FlowOptimizerPort, MetricsPort, RuleManagementPort};
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
                if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                    req_dtos.push(FlowRequestDto {
                        url: url.to_string(),
                        method: item
                            .get("method")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        resource_type: item
                            .get("resource_type")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        size_bytes: item.get("size_bytes").and_then(|v| v.as_u64()),
                    });
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
