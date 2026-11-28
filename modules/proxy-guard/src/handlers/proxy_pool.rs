//! Upstream Multi-Proxy Pool HTTP / JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.
//! Provides endpoints to register, list, query, and balance multiple upstream proxies.

use crate::domain::{
    ProxyRotationStrategy, ProxySelectionQuery, StickySessionConfig, UpstreamProxy,
};
use crate::ports::ProxyPoolManagementPort;
use crate::rpc::{make_error_resp, make_json_resp};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

/// GET /proxies/list
pub fn handle_list_proxies(service: &ProxyGuardService) -> Value {
    let list: Vec<_> = service
        .list_proxies()
        .into_iter()
        .map(|p| p.to_masked())
        .collect();
    let strategy = service.get_strategy();
    make_json_resp(
        200,
        json!({
            "status": "ok",
            "strategy": strategy,
            "total_proxies": list.len(),
            "proxies": list
        }),
    )
}

/// POST /proxies/add
/// Supports adding a single proxy or a batch of multiple proxies at once.
pub fn handle_add_proxy(body: &Value, service: &ProxyGuardService) -> Value {
    // Check if batch array provided: { "proxies": [ "...", { "url": "..." } ] }
    if let Some(arr) = body.get("proxies").and_then(|v| v.as_array()) {
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
                    let pool_name = obj
                        .get("pool_name")
                        .or_else(|| obj.get("pool"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    match StickySessionConfig::parse_optional_from_json(item) {
                        Ok(sticky) => UpstreamProxy::parse(url, name, country, tags)
                            .map(|p| p.with_pool_and_sticky(pool_name, sticky)),
                        Err(e) => Err(e),
                    }
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

        return make_json_resp(
            200,
            json!({
                "status": "ok",
                "added_count": added_ids.len(),
                "added_ids": added_ids,
                "errors": errors
            }),
        );
    }

    // Single proxy addition
    let url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return make_error_resp(400, "Missing required 'url' field"),
    };
    let name = body.get("name").and_then(|v| v.as_str());
    let country = body.get("country").and_then(|v| v.as_str());
    let tags = body.get("tags").and_then(|v| v.as_array()).map(|t| {
        t.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    });
    let pool_name = body
        .get("pool_name")
        .or_else(|| body.get("pool"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let sticky = match StickySessionConfig::parse_optional_from_json(body) {
        Ok(cfg) => cfg,
        Err(e) => return make_error_resp(400, &e),
    };

    match UpstreamProxy::parse(url, name, country, tags)
        .map(|p| p.with_pool_and_sticky(pool_name, sticky))
    {
        Ok(proxy) => match service.add_proxy(proxy) {
            Ok(id) => make_json_resp(
                200,
                json!({
                    "status": "ok",
                    "message": "Proxy registered successfully",
                    "id": id
                }),
            ),
            Err(e) => make_error_resp(500, &e),
        },
        Err(e) => make_error_resp(400, &format!("Invalid proxy definition: {e}")),
    }
}

/// POST /proxies/remove
pub fn handle_remove_proxy(body: &Value, service: &ProxyGuardService) -> Value {
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) => i,
        None => return make_error_resp(400, "Missing required 'id' field"),
    };

    match service.remove_proxy(id) {
        Ok(_) => make_json_resp(
            200,
            json!({
                "status": "ok",
                "message": format!("Proxy '{id}' removed successfully")
            }),
        ),
        Err(e) => make_error_resp(404, &e),
    }
}

/// GET /proxies/next
/// Selects an upstream proxy according to pool rotation strategy and optional query filters.
pub fn handle_get_next_proxy(query_val: &Value, service: &ProxyGuardService) -> Value {
    let strategy_override = query_val
        .get("strategy")
        .and_then(|v| v.as_str())
        .and_then(ProxyRotationStrategy::parse);

    let exclude_ids = parse_exclude_ids(query_val);

    let query = ProxySelectionQuery {
        session_id: query_val
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        country: query_val
            .get("country")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        tag: query_val
            .get("tag")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        pool: query_val
            .get("pool")
            .or_else(|| query_val.get("pool_name"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        preferred_id: query_val
            .get("preferred_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        strategy: strategy_override,
        exclude_ids,
    };

    let use_interceptor = query_val
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|m| {
            m.eq_ignore_ascii_case("intercept")
                || m.eq_ignore_ascii_case("guard")
                || m.eq_ignore_ascii_case("optimized")
        })
        .or_else(|| query_val.get("interceptor").and_then(|v| v.as_bool()))
        .unwrap_or(false);

    match service.select_proxy(&query) {
        Some(proxy) => {
            let mut playwright_cfg = proxy.to_playwright_config();
            let applied_strategy = strategy_override
                .unwrap_or_else(|| service.get_strategy())
                .as_str();

            if use_interceptor {
                playwright_cfg.server = "http://132.226.92.23:3128".to_string();
            }

            make_json_resp(
                200,
                json!({
                    "status": "ok",
                    "strategy_applied": applied_strategy,
                    "proxy": proxy,
                    "playwright_config": playwright_cfg,
                    "interceptor_server": "http://132.226.92.23:3128",
                    "recommended_launch_args": [
                        "--disable-background-networking",
                        "--disable-component-update",
                        "--disable-default-apps",
                        "--no-default-browser-check"
                    ],
                    "puppeteer_args": [
                        format!("--proxy-server={}", playwright_cfg.server)
                    ]
                }),
            )
        }
        None => {
            let msg = match &query.country {
                Some(c) => format!("No healthy upstream proxy available for country '{c}' in pool"),
                None => "No healthy upstream proxy available in pool".to_string(),
            };
            make_error_resp(404, &msg)
        }
    }
}

/// POST /proxies/strategy
pub fn handle_set_strategy(body: &Value, service: &ProxyGuardService) -> Value {
    let strat_str =
        match body.get("strategy").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return make_error_resp(
                400,
                "Missing 'strategy' field (round_robin, random, sticky_session, lowest_failures)",
            ),
        };

    let strat = match strat_str.to_ascii_lowercase().as_str() {
        "round_robin" => ProxyRotationStrategy::RoundRobin,
        "random" => ProxyRotationStrategy::Random,
        "sticky_session" | "sticky" => ProxyRotationStrategy::StickySession,
        "lowest_failures" | "health" => ProxyRotationStrategy::LowestFailures,
        _ => return make_error_resp(400, "Unknown rotation strategy"),
    };

    match service.set_strategy(strat) {
        Ok(_) => make_json_resp(
            200,
            json!({
                "status": "ok",
                "message": format!("Strategy updated to {:?}", strat)
            }),
        ),
        Err(e) => make_error_resp(500, &e),
    }
}

/// POST /proxies/report
/// Records proxy health feedback (success or failure/block) to guide `lowest_failures` routing.
pub fn handle_report_proxy(body: &Value, service: &ProxyGuardService) -> Value {
    let id = match body.get("id").and_then(|v| v.as_str()) {
        Some(i) if !i.trim().is_empty() => i.trim(),
        _ => return make_error_resp(400, "Missing required 'id' field"),
    };
    let success = match body.get("success").and_then(|v| v.as_bool()) {
        Some(b) => b,
        None => return make_error_resp(400, "Missing required boolean 'success' field"),
    };

    let status_code = body.get("status_code").and_then(|v| v.as_u64());
    let is_auth_error = body
        .get("auth_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || status_code == Some(407);

    if !success && is_auth_error {
        service.record_proxy_auth_failure(id);
    } else {
        service.record_proxy_result(id, success);
    }
    make_json_resp(
        200,
        json!({
            "status": "ok",
            "id": id,
            "success": success,
            "quarantined": !success && is_auth_error,
            "message": "Proxy health result recorded"
        }),
    )
}

fn parse_exclude_ids(val: &Value) -> Vec<String> {
    if let Some(arr) = val.get("exclude_ids").and_then(|v| v.as_array()) {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(csv) = val
        .get("exclude_ids")
        .or_else(|| val.get("exclude_id"))
        .and_then(|v| v.as_str())
    {
        return csv
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    Vec::new()
}
