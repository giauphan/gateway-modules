//! Standalone Native Rust stdio-stream entrypoint for Daily News.
//!
//! Frameworks & Drivers layer: Reads JSON requests from stdin, dispatches
//! to application service, and writes JSON responses to stdout.

use daily_news::fetcher::LiveFeedFetcher;
use daily_news::redis::RedisFeedProvider;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::FileStorageAdapter;
use daily_news::ui::render_dashboard_html;
use daily_news::{RawNewsItem, StoragePort};
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

fn extract_topics(body: &Value) -> Option<Vec<String>> {
    if let Some(arr) = body["topics"].as_array() {
        let items: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    if let Some(s) = body["topics"].as_str().or_else(|| body["topic"].as_str()) {
        let items: Vec<String> = s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    None
}

fn extract_date_range(body: &Value) -> (Option<String>, Option<String>) {
    let from = body["from_date"]
        .as_str()
        .or_else(|| body["since"].as_str())
        .or_else(|| body["start_date"].as_str())
        .or_else(|| body["from"].as_str())
        .map(str::to_string);

    let to = body["to_date"]
        .as_str()
        .or_else(|| body["until"].as_str())
        .or_else(|| body["end_date"].as_str())
        .or_else(|| body["to"].as_str())
        .map(str::to_string);

    (from, to)
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
        let (from_date, to_date) = extract_date_range(&body);
        let topics = extract_topics(&body);
        let curated: Option<Vec<RawNewsItem>> = body["curated_candidates"]
            .as_array()
            .map(|arr| serde_json::from_value(json!(arr)).unwrap_or_default());
        let sources: Option<Vec<String>> = body["sources"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());

        match service.discover(
            limit,
            curated,
            sources,
            from_date.as_deref(),
            to_date.as_deref(),
            topics.as_deref(),
        ) {
            Ok(result) => return make_json_resp(200, json!(result)),
            Err((status_code, err_msg)) => {
                return make_json_resp(
                    400,
                    json!({
                        "tier": "none",
                        "status": status_code,
                        "candidates": [],
                        "reason": err_msg
                    }),
                );
            }
        }
    }

    // 6. Candidates Endpoint
    if path.ends_with("/api/v1/candidates") {
        if method == "POST" && body.get("items").is_some() {
            let raw_items: Vec<RawNewsItem> = serde_json::from_value(body["items"].clone()).unwrap_or_default();
            let (from_date, to_date) = extract_date_range(&body);
            let from_epoch = from_date.as_deref().and_then(|d| daily_news::date::parse_boundary_datetime(d, false).ok());
            let to_epoch = to_date.as_deref().and_then(|d| daily_news::date::parse_boundary_datetime(d, true).ok());
            let topics = extract_topics(&body);

            let ranked = service.filter_and_score(&raw_items, from_epoch, to_epoch, topics.as_deref(), limit);
            let count = ranked.len();
            return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
        }

        let ranked = service.get_candidates(limit);
        let count = ranked.len();
        return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
    }

    // 7. Feed Ingestion Endpoint
    if path.ends_with("/api/v1/feed") {
        let sources: Option<Vec<String>> = body["sources"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
        let feed = if let Some(ref s) = sources {
            service.get_feed_sources(s, limit)
        } else {
            service.get_feed(limit)
        };
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
            let mut cfg = service.get_config();
            if let Some(url) = body["newsnow_url"].as_str() {
                cfg.newsnow_url = url.to_string();
            }
            if let Some(h) = body["freshness_window_hours"].as_f64() {
                cfg.freshness_window_hours = h;
            }
            if let Some(d) = body["max_age_days"].as_u64() {
                cfg.max_age_days = d as u32;
            }
            if let Some(b) = body["require_latest_news"].as_bool() {
                cfg.require_latest_news = b;
            }
            if let Some(re) = body["redis_enabled"].as_bool() {
                cfg.redis_enabled = re;
            }
            if let Some(ru) = body["redis_url"].as_str() {
                cfg.redis_url = ru.to_string();
            }
            if let Some(rk) = body["redis_key"].as_str() {
                cfg.redis_key = rk.to_string();
            }
            if let Some(rt) = body["redis_type"].as_str() {
                cfg.redis_type = rt.to_string();
            }

            match service.update_config(cfg) {
                Ok(updated) => {
                    return make_json_resp(200, json!({ "status": "updated", "config": updated }));
                }
                Err(e) => {
                    return make_json_resp(500, json!({ "error": e }));
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
    let redis_provider = if initial_config.redis_enabled {
        Some(RedisFeedProvider::new(
            &initial_config.redis_url,
            &initial_config.redis_key,
            &initial_config.redis_type,
        ))
    } else {
        None
    };

    let mut fetcher = LiveFeedFetcher::new(Some(initial_config.newsnow_url));
    if let Some(rp) = redis_provider {
        fetcher = fetcher.with_redis(rp);
    }
    let fetcher = Arc::new(fetcher);
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
