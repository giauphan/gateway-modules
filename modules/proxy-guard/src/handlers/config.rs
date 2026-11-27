//! Configuration Route Handlers for Gateway Settings UI and API.
//!
//! Clean Architecture: Interface Adapters Layer.

use crate::domain::{
    parse_country_input, DefaultRoute, ProxyRotationStrategy, RuleCategory, UpstreamProxy,
};
use crate::http_cache::HttpCacheConfig;
use crate::ports::{MetricsPort, ProxyPoolManagementPort, RuleManagementPort};
use crate::rpc::{extract_request_body, make_error_resp, make_json_resp, make_text_resp};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_config_route(req: &Value, method: &str, service: &ProxyGuardService) -> Value {
    match method {
        "GET" => {
            if wants_html(req) {
                let html = crate::ui::render_dashboard_html(service);
                make_text_resp(200, "text/html; charset=utf-8", &html)
            } else {
                make_json_resp(200, get_config_json(service))
            }
        }
        "POST" => match extract_request_body(req) {
            Ok(body) => update_config(&body, service),
            Err(e) => make_error_resp(400, &e),
        },
        _ => make_error_resp(405, "Method Not Allowed on /config"),
    }
}

pub fn get_config_json(service: &ProxyGuardService) -> Value {
    let proxies = service.list_proxies();
    let upstream_proxy_pool = proxies
        .iter()
        .map(|p| p.pool_line())
        .collect::<Vec<_>>()
        .join("\n");
    let upstream_proxy_url = if proxies.len() == 1 {
        proxies.first().map(|p| p.url.as_str()).unwrap_or("")
    } else {
        ""
    };
    let default_country = proxies
        .iter()
        .find_map(|p| p.country.as_deref())
        .unwrap_or("VN");
    let append_cc_to_username = proxies.is_empty()
        || proxies.iter().any(|p| {
            p.auth
                .as_ref()
                .map(|a| a.username.to_ascii_lowercase().contains("-cc-"))
                .unwrap_or(false)
        });
    let strategy = match service.get_strategy() {
        ProxyRotationStrategy::RoundRobin => "round_robin",
        ProxyRotationStrategy::Random => "random",
        ProxyRotationStrategy::StickySession => "sticky_session",
        ProxyRotationStrategy::LowestFailures => "lowest_failures",
    };

    let rules = service.get_rules();
    let has_enabled = |cat: RuleCategory| rules.iter().any(|r| r.category == cat && r.enabled);

    json!({
        "status": "ok",
        "upstream_proxy_pool": upstream_proxy_pool,
        "default_country": default_country,
        "append_cc_to_username": append_cc_to_username,
        "rotation_strategy": strategy,
        "upstream_proxy_url": upstream_proxy_url,
        "proxy_listener_port": 3128,
        "block_analytics": has_enabled(RuleCategory::Analytics),
        "block_ads": has_enabled(RuleCategory::Advertising),
        "block_telemetry": has_enabled(RuleCategory::Telemetry),
        "block_media": has_enabled(RuleCategory::HeavyMedia),
        "block_fonts": has_enabled(RuleCategory::Font),
        "default_route": service.get_default_route(),
        "cache": service.http_cache().config(),
        "cost_per_gb_usd": service.get_metrics().cost_per_gb_usd
    })
}

fn apply_default_country(
    mut proxy: UpstreamProxy,
    default_cc: Option<&str>,
    force_cc_username: bool,
) -> UpstreamProxy {
    if force_cc_username {
        if let Some(cc) = default_cc.or(proxy.country.as_deref()) {
            return proxy.with_cc_username(cc);
        }
    } else if proxy.country.is_none() {
        proxy.country = default_cc.map(|s| s.to_string());
    }
    proxy
}

fn update_config(body: &Value, service: &ProxyGuardService) -> Value {
    let parsed_country = body
        .get("default_country")
        .or_else(|| body.get("country"))
        .and_then(|v| v.as_str())
        .and_then(parse_country_input);
    let append_flag = body
        .get("append_cc_to_username")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let (default_country, force_cc_username) = match parsed_country {
        Some((cc, from_prefix)) => (Some(cc), from_prefix || append_flag),
        None => (None, append_flag),
    };

    // 1. Multi-proxy pool (authoritative when non-empty)
    let mut pool_populated = false;
    if let Some(pool_raw) = body.get("upstream_proxy_pool").and_then(|v| v.as_str()) {
        let mut new_proxies = Vec::new();
        for line in pool_raw.split(['\n', ',', ';']) {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Ok(proxy) = UpstreamProxy::parse(trimmed, None, None, None) {
                    new_proxies.push(apply_default_country(
                        proxy,
                        default_country.as_deref(),
                        force_cc_username,
                    ));
                }
            }
        }
        pool_populated = !new_proxies.is_empty();
        let _ = service.replace_proxies(new_proxies);
    }

    // 2. Fallback single proxy url (only used when multi-proxy pool is empty/absent)
    if !pool_populated {
        if let Some(single_url) = body.get("upstream_proxy_url").and_then(|v| v.as_str()) {
            let trimmed = single_url.trim();
            if !trimmed.is_empty() {
                if let Ok(proxy) = UpstreamProxy::parse(trimmed, None, None, None) {
                    let _ = service.add_proxy(apply_default_country(
                        proxy,
                        default_country.as_deref(),
                        force_cc_username,
                    ));
                }
            }
        }
    }

    // 3. Rotation strategy
    if let Some(strat_str) = body.get("rotation_strategy").and_then(|v| v.as_str()) {
        let strat = match strat_str.to_ascii_lowercase().as_str() {
            "round_robin" => ProxyRotationStrategy::RoundRobin,
            "random" => ProxyRotationStrategy::Random,
            "sticky_session" | "sticky" => ProxyRotationStrategy::StickySession,
            "lowest_failures" | "health" => ProxyRotationStrategy::LowestFailures,
            _ => ProxyRotationStrategy::RoundRobin,
        };
        let _ = service.set_strategy(strat);
    }

    // 4. Rule toggles
    let toggles = [
        ("block_analytics", RuleCategory::Analytics),
        ("block_ads", RuleCategory::Advertising),
        ("block_telemetry", RuleCategory::Telemetry),
        ("block_media", RuleCategory::HeavyMedia),
        ("block_fonts", RuleCategory::Font),
    ];
    for (key, cat) in toggles {
        if let Some(b) = body.get(key).and_then(|v| v.as_bool()) {
            let _ = service.toggle_category(cat, b);
        }
    }

    // 5. Cost per GB
    if let Some(cost) = body.get("cost_per_gb_usd").and_then(|v| v.as_f64()) {
        let _ = service.set_cost_per_gb(cost);
    }

    // 6. Optional default route & HTTP response cache config
    if let Some(dr_val) = body.get("default_route") {
        let route = match DefaultRoute::from_json_value(dr_val) {
            Ok(r) => r,
            Err(e) => return make_error_resp(400, &e),
        };
        if let Err(e) = service.set_default_route(route) {
            return make_error_resp(500, &e);
        }
    }
    if let Some(cache_val) = body.get("cache") {
        if let Ok(cache_cfg) = serde_json::from_value::<HttpCacheConfig>(cache_val.clone()) {
            service.configure_cache(cache_cfg);
        }
    }

    make_json_resp(
        200,
        json!({
            "status": "success",
            "message": "Proxy Guard configuration updated successfully",
            "config": get_config_json(service)
        }),
    )
}

fn wants_html(req: &Value) -> bool {
    let headers = match req.get("headers") {
        Some(h) => h,
        None => return false,
    };

    if let Some(map) = headers.as_object() {
        if let Some(accept) = map.get("accept").and_then(|v| v.as_str()) {
            return accept.contains("text/html");
        }
    }

    if let Some(arr) = headers.as_array() {
        for entry in arr {
            if let Some(pair) = entry.as_array() {
                if pair.len() == 2 && pair[0].as_str() == Some("accept") {
                    if let Some(val) = pair[1].as_str() {
                        return val.contains("text/html");
                    }
                }
            }
        }
    }

    false
}
