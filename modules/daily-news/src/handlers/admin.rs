//! Admin and configuration route handlers (health, models, ui, flowchart, config).

use crate::rpc::{make_html_resp, make_json_resp};
use crate::service::DailyNewsService;
use crate::ui::render_dashboard_html;
use serde_json::{json, Value};

pub fn handle_health(service: &DailyNewsService) -> Value {
    let cfg = service.get_config();
    make_json_resp(
        200,
        json!({
            "status": "healthy",
            "module": "daily-news",
            "version": env!("CARGO_PKG_VERSION"),
            "engine": "rust-native",
            "consumed_count": service.consumed_count(),
            "cached_candidates": service.cached_candidates_count(),
            "newsnow_url": cfg.newsnow_url,
            "redis_enabled": cfg.redis_enabled,
            "reddit_enabled": cfg.reddit_enabled,
            "reddit_subreddits": cfg.reddit_subreddits
        }),
    )
}

pub fn handle_models() -> Value {
    make_json_resp(
        200,
        json!({
            "object": "list",
            "data": [{
                "id": "daily-news",
                "object": "model",
                "created": 1793618000,
                "owned_by": "daily-news-rs"
            }]
        }),
    )
}

pub fn handle_ui(service: &DailyNewsService, limit: usize) -> Value {
    let cfg = service.get_config();
    let candidates = service.get_candidates(limit.max(10));
    let html = render_dashboard_html(
        env!("CARGO_PKG_VERSION"),
        "rust-native",
        &cfg,
        service.consumed_count(),
        &candidates,
    );
    make_html_resp(200, &html)
}

pub fn handle_flowchart() -> Value {
    make_html_resp(200, include_str!("../../docs/flowchart.html"))
}

pub fn handle_config(service: &DailyNewsService, method: &str, body: &Value) -> Value {
    if method == "POST" {
        if !body.is_object() {
            return make_json_resp(
                400,
                json!({ "error": "Config payload must be a JSON object" }),
            );
        }
        let mut cfg = service.get_config();
        if let Err(e) = cfg.apply_updates_validated(body) {
            return make_json_resp(400, json!({ "error": e }));
        }
        return match service.update_config(cfg) {
            Ok(updated) => make_json_resp(
                200,
                json!({ "status": "updated", "config": updated.to_sanitized_value() }),
            ),
            Err(e) => make_json_resp(500, json!({ "error": e })),
        };
    }
    let current = service.get_config();
    make_json_resp(200, current.to_sanitized_value())
}
