//! Rules & Script Export HTTP/JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.

use crate::domain::{DefaultRoute, RuleCategory};
use crate::http_cache::HttpCacheConfig;
use crate::ports::{FlowOptimizerPort, MetricsPort, RuleManagementPort};
use crate::rpc::{make_error_resp, make_json_resp, make_text_resp};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_get_rules(service: &ProxyGuardService) -> Value {
    let rules = service.get_rules();
    make_json_resp(200, json!({ "rules": rules }))
}

pub fn handle_update_rules(body: &Value, service: &ProxyGuardService) -> Value {
    if let Some(b) = body.get("block_analytics").and_then(|v| v.as_bool()) {
        if let Err(e) = service.toggle_category(RuleCategory::Analytics, b) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(b) = body.get("block_ads").and_then(|v| v.as_bool()) {
        if let Err(e) = service.toggle_category(RuleCategory::Advertising, b) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(b) = body.get("block_telemetry").and_then(|v| v.as_bool()) {
        if let Err(e) = service.toggle_category(RuleCategory::Telemetry, b) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(b) = body.get("block_media").and_then(|v| v.as_bool()) {
        if let Err(e) = service.toggle_category(RuleCategory::HeavyMedia, b) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(b) = body.get("block_fonts").and_then(|v| v.as_bool()) {
        if let Err(e) = service.toggle_category(RuleCategory::Font, b) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(arr) = body.get("add_whitelist_domains").and_then(|v| v.as_array()) {
        for d in arr.iter().filter_map(|v| v.as_str()) {
            if let Err(e) = service.add_whitelist_domain(d) {
                return make_error_resp(500, &e);
            }
        }
    }
    if let Some(arr) = body.get("add_blocked_domains").and_then(|v| v.as_array()) {
        for d in arr.iter().filter_map(|v| v.as_str()) {
            if let Err(e) = service.add_blocked_domain(d, None) {
                return make_error_resp(500, &e);
            }
        }
    }
    if let Some(arr) = body
        .get("add_direct_bypass_domains")
        .and_then(|v| v.as_array())
    {
        for d in arr.iter().filter_map(|v| v.as_str()) {
            if let Err(e) = service.add_direct_bypass_domain(d, None) {
                return make_error_resp(500, &e);
            }
        }
    }
    if let Some(arr) = body.get("add_pool_routes").and_then(|v| v.as_array()) {
        for item in arr {
            let domain = item.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            let pool = item.get("pool").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str());
            if !domain.is_empty() && !pool.is_empty() {
                if let Err(e) = service.add_pool_route_domain(domain, pool, name) {
                    return make_error_resp(400, &e);
                }
            }
        }
    }
    if let Some(dr_val) = body.get("default_route") {
        let parsed = if let Some(s) = dr_val.as_str() {
            match s.trim().to_ascii_lowercase().as_str() {
                "direct" | "bypass_direct" => Some(DefaultRoute::Direct),
                "forward_upstream" | "upstream" => Some(DefaultRoute::ForwardUpstream),
                other if !other.is_empty() => Some(DefaultRoute::Pool(other.to_string())),
                _ => None,
            }
        } else {
            serde_json::from_value::<DefaultRoute>(dr_val.clone()).ok()
        };
        if let Some(route) = parsed {
            if let Err(e) = service.set_default_route(route) {
                return make_error_resp(500, &e);
            }
        }
    }
    if let Some(cache_val) = body.get("cache") {
        if let Ok(cache_cfg) = serde_json::from_value::<HttpCacheConfig>(cache_val.clone()) {
            service.configure_cache(cache_cfg);
        }
    }
    if let Some(rate) = body.get("cost_per_gb_usd").and_then(|v| v.as_f64()) {
        if let Err(e) = service.set_cost_per_gb(rate) {
            return make_error_resp(400, &e);
        }
    }

    make_json_resp(200, json!({ "status": "ok", "message": "Rules updated" }))
}

pub fn handle_get_script(target: &str, service: &ProxyGuardService) -> Value {
    match service.generate_client_script(target) {
        Ok(code) => {
            let content_type = if target.ends_with("_py") {
                "text/x-python; charset=utf-8"
            } else {
                "application/javascript; charset=utf-8"
            };
            make_text_resp(200, content_type, &code)
        }
        Err(err) => make_error_resp(400, &err),
    }
}
