//! Standalone Native Rust stdio-stream entrypoint for Daily News.
//!
//! Frameworks & Drivers layer: Reads JSON requests from stdin, dispatches
//! to application service, and writes JSON responses to stdout.

use daily_news::fetcher::LiveFeedFetcher;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::FileStorageAdapter;
use daily_news::ui::render_dashboard_html;
use daily_news::StoragePort;
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Arc;

fn make_json_resp(status: u16, data: Value) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": "application/json",
            "x-module-engine": "rust-native"
        },
        "body": serde_json::to_string(&data).unwrap_or_default()
    })
}

fn make_html_resp(status: u16, html: &str) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": "text/html; charset=utf-8",
            "x-module-engine": "rust-native"
        },
        "body": html
    })
}

fn handle_request(req: Value, service: &DailyNewsService) -> Value {
    let raw_path = req["path"].as_str().unwrap_or("");
    let (path, query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (raw_path.trim_end_matches('/'), ""),
    };

    let method = req["method"]
        .as_str()
        .unwrap_or("GET")
        .to_ascii_uppercase();

    let mut body: Value = if let Some(b) = req["body"].as_str() {
        serde_json::from_str(b).unwrap_or_else(|_| json!({}))
    } else if req["body"].is_object() {
        req["body"].clone()
    } else {
        json!({})
    };

    // Merge query parameters into body
    if !query.is_empty() {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if body.get(k).is_none() {
                    if let Ok(num) = v.parse::<i64>() {
                        body[k] = json!(num);
                    } else {
                        body[k] = json!(v);
                    }
                }
            }
        }
    }

    let limit = body["limit"]
        .as_u64()
        .or_else(|| body["count"].as_u64())
        .unwrap_or(5) as usize;

    // 1. Health Endpoint
    if path.ends_with("/health") {
        let cfg = service.get_config();
        return make_json_resp(
            200,
            json!({
                "status": "healthy",
                "module": "daily-news",
                "version": "1.3.0",
                "engine": "rust-native",
                "consumed_count": service.consumed_count(),
                "cached_candidates": service.cached_candidates_count(),
                "newsnow_url": cfg.newsnow_url,
                "redis_enabled": cfg.redis_enabled
            }),
        );
    }

    // 2. Models Endpoint
    if path.ends_with("/models") {
        return make_json_resp(
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
        );
    }

    // 3. UI Dashboard Endpoint
    if path.ends_with("/ui") || path.ends_with("/custom/daily-news") {
        let cfg = service.get_config();
        let candidates = service.get_candidates(limit.max(10));
        let html = render_dashboard_html(
            "1.3.0",
            "rust-native",
            &cfg,
            service.consumed_count(),
            &candidates,
        );
        return make_html_resp(200, &html);
    }

    // 4. Chat Completions
    if path.ends_with("/chat/completions") {
        let content = service.chat_completion(limit);
        return make_json_resp(
            200,
            json!({
                "id": format!("chatcmpl-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                "object": "chat.completion",
                "model": "daily-news",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": content },
                    "finish_reason": "stop"
                }]
            }),
        );
    }

    // 5. Tiered Content Discovery
    if path.ends_with("/api/v1/discover") {
        let from_date = body["from_date"].as_str();
        let to_date = body["to_date"].as_str();
        let topics: Option<Vec<String>> = body["topics"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());

        let result = service.discover(limit, from_date, to_date, topics.as_deref());
        return make_json_resp(200, json!(result));
    }

    // 6. Candidates Endpoint
    if path.ends_with("/api/v1/candidates") {
        let ranked = service.get_candidates(limit);
        let count = ranked.len();
        return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
    }

    // 7. Feed Ingestion Endpoint
    if path.ends_with("/api/v1/feed") {
        let feed = service.get_feed(limit);
        let count = feed.len();
        return make_json_resp(200, json!({ "count": count, "items": feed }));
    }

    // 8. Consume Candidates
    if path.ends_with("/api/v1/consume") {
        let ids: Vec<String> = if let Some(arr) = body["candidate_ids"].as_array() {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        } else if let Some(single) = body["candidate_id"].as_str() {
            vec![single.to_string()]
        } else {
            Vec::new()
        };

        match service.consume(&ids) {
            Ok(count) => {
                return make_json_resp(200, json!({ "status": "consumed", "count": count }));
            }
            Err(e) => {
                return make_json_resp(500, json!({ "error": e }));
            }
        }
    }

    // 9. Configuration Management
    if path.ends_with("/api/v1/config") {
        if method == "POST" {
            let new_config: Result<daily_news::domain::ModuleConfig, _> =
                serde_json::from_value(body.clone());
            match new_config {
                Ok(cfg) => match service.update_config(cfg) {
                    Ok(updated) => {
                        return make_json_resp(200, json!({ "status": "updated", "config": updated }));
                    }
                    Err(e) => {
                        return make_json_resp(500, json!({ "error": e }));
                    }
                },
                Err(e) => {
                    return make_json_resp(400, json!({ "error": format!("Invalid config schema: {e}") }));
                }
            }
        } else {
            let current = service.get_config();
            return make_json_resp(200, json!(current));
        }
    }

    make_json_resp(404, json!({ "error": format!("Route not found: {raw_path}") }))
}

fn main() {
    let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let storage = Arc::new(FileStorageAdapter::new(&base_dir));
    let initial_config = storage.load_config();
    let fetcher = Arc::new(LiveFeedFetcher::new(Some(initial_config.newsnow_url)));
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage, scorer);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = make_json_resp(400, json!({ "error": format!("Invalid JSON: {e}") }));
                println!("{}", serde_json::to_string(&err_resp).unwrap_or_default());
                continue;
            }
        };

        let resp = handle_request(req, &service);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}
