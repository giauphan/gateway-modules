//! Upstream Multi-Proxy Pool HTTP / JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.
//! Provides endpoints to register, list, query, and balance multiple upstream proxies.

use crate::domain::{ProxyRotationStrategy, ProxySelectionQuery, UpstreamProxy};
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

    match UpstreamProxy::parse(url, name, country, tags) {
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
        preferred_id: query_val
            .get("preferred_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    match service.select_proxy(&query) {
        Some(proxy) => {
            let playwright_cfg = proxy.to_playwright_config();
            make_json_resp(
                200,
                json!({
                    "status": "ok",
                    "proxy": proxy,
                    "playwright_config": playwright_cfg,
                    "puppeteer_args": [
                        format!("--proxy-server={}", playwright_cfg.server)
                    ]
                }),
            )
        }
        None => make_error_resp(404, "No healthy upstream proxy available in pool"),
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
