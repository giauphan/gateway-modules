//! Upstream Proxy Pool & Chat MCP Tool Handlers.
//!
//! Clean Architecture: Interface Adapters (Submodule of `mcp`).

use super::{make_mcp_error, make_mcp_success};
use crate::domain::{
    ProxyRotationStrategy, ProxySelectionQuery, StickySessionConfig, UpstreamProxy,
};
use crate::ports::ProxyPoolManagementPort;
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

fn parse_proxy_item(item: &Value, idx: usize) -> Result<UpstreamProxy, String> {
    match item {
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
            let pool_name = obj
                .get("pool_name")
                .or_else(|| obj.get("pool"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let sticky = StickySessionConfig::parse_optional_from_json(item)?;
            UpstreamProxy::parse(url, name, country, tags)
                .map(|p| p.with_pool_and_sticky(pool_name, sticky))
        }
        _ => Err(format!("Item at index {idx} has invalid format")),
    }
}

pub(super) fn tool_add_upstream_proxy(arguments: &Value, service: &ProxyGuardService) -> Value {
    if let Some(arr) = arguments.get("proxies").and_then(|v| v.as_array()) {
        let mut added_ids = Vec::with_capacity(arr.len());
        let mut errors = Vec::new();

        for (idx, item) in arr.iter().enumerate() {
            match parse_proxy_item(item, idx).and_then(|p| service.add_proxy(p)) {
                Ok(id) => added_ids.push(id),
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
        let pool_name = arguments
            .get("pool_name")
            .or_else(|| arguments.get("pool"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let sticky = match StickySessionConfig::parse_optional_from_json(arguments) {
            Ok(cfg) => cfg,
            Err(e) => return make_mcp_error(&e),
        };

        match UpstreamProxy::parse(url, name, country, tags)
            .map(|p| p.with_pool_and_sticky(pool_name, sticky))
        {
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

pub(super) fn tool_list_upstream_proxies(service: &ProxyGuardService) -> Value {
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

pub(super) fn tool_get_next_proxy(arguments: &Value, service: &ProxyGuardService) -> Value {
    let strategy_override = arguments
        .get("strategy")
        .and_then(|v| v.as_str())
        .and_then(ProxyRotationStrategy::parse);

    let exclude_ids: Vec<String> = arguments
        .get("exclude_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

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
        pool: arguments
            .get("pool")
            .or_else(|| arguments.get("pool_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        preferred_id: arguments
            .get("preferred_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        strategy: strategy_override,
        exclude_ids,
    };

    let mask_credentials = arguments
        .get("mask_credentials")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match service.select_proxy(&query) {
        Some(proxy) => {
            let output_proxy = if mask_credentials {
                proxy.to_masked()
            } else {
                proxy
            };
            let playwright_cfg = output_proxy.to_playwright_config();
            let applied_strategy = strategy_override
                .unwrap_or_else(|| service.get_strategy())
                .as_str();
            make_mcp_success(&json!({
                "status": "success",
                "strategy_applied": applied_strategy,
                "proxy": output_proxy,
                "playwright_config": playwright_cfg,
                "curl_arg": format!("-x {}", output_proxy.url)
            }))
        }
        None => {
            let msg = match &query.country {
                Some(c) => format!("No available upstream proxy for country '{c}' in pool"),
                None => "No available upstream proxy in pool".to_string(),
            };
            make_mcp_error(&msg)
        }
    }
}

pub(super) fn tool_remove_upstream_proxy(arguments: &Value, service: &ProxyGuardService) -> Value {
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

pub(super) fn tool_set_rotation_strategy(arguments: &Value, service: &ProxyGuardService) -> Value {
    let strat_str = match arguments.get("strategy").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return make_mcp_error("Missing parameter 'strategy'"),
    };

    let strat =
        match ProxyRotationStrategy::parse(strat_str) {
            Some(s) => s,
            None => return make_mcp_error(
                "Invalid strategy. Options: round_robin, random, sticky_session, lowest_failures",
            ),
        };

    match service.set_strategy(strat) {
        Ok(_) => make_mcp_success(&json!({
            "status": "success",
            "message": format!("Strategy set to {:?}", strat)
        })),
        Err(e) => make_mcp_error(&e),
    }
}

pub(super) fn tool_chat(arguments: &Value, service: &ProxyGuardService) -> Value {
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
