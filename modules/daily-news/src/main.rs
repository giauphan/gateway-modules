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
        let items: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    body["topics"]
        .as_str()
        .or_else(|| body["topic"].as_str())
        .and_then(|s| {
            let items: Vec<String> = s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(items)
            }
        })
}

fn extract_sources(body: &Value) -> Option<Vec<String>> {
    if let Some(arr) = body["sources"].as_array().or_else(|| body["source"].as_array()) {
        let items: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    body["sources"]
        .as_str()
        .or_else(|| body["source"].as_str())
        .and_then(|s| {
            let items: Vec<String> = s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(items)
            }
        })
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

fn parse_request_body(req: &Value, query: &str) -> Value {
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
    body
}

fn handle_health(service: &DailyNewsService) -> Value {
    let cfg = service.get_config();
    make_json_resp(
        200,
        json!({
            "status": "healthy",
            "module": "daily-news",
            "version": "1.5.0",
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

fn handle_models() -> Value {
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

fn handle_ui(service: &DailyNewsService, limit: usize) -> Value {
    let cfg = service.get_config();
    let candidates = service.get_candidates(limit.max(10));
    let html = render_dashboard_html(
        "1.5.0",
        "rust-native",
        &cfg,
        service.consumed_count(),
        &candidates,
    );
    make_html_resp(200, &html)
}

fn handle_chat_completions(service: &DailyNewsService, limit: usize) -> Value {
    let content = service.chat_completion(limit);
    make_json_resp(
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
    )
}

fn handle_discover(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
    let (from_date, to_date) = extract_date_range(body);
    let topics = extract_topics(body);
    let curated: Option<Vec<RawNewsItem>> = body["curated_candidates"]
        .as_array()
        .map(|arr| serde_json::from_value(json!(arr)).unwrap_or_default());
    let sources = extract_sources(body);

    match service.discover(
        limit,
        curated,
        sources,
        from_date.as_deref(),
        to_date.as_deref(),
        topics.as_deref(),
    ) {
        Ok(result) => make_json_resp(200, json!(result)),
        Err((status_code, err_msg)) => make_json_resp(
            400,
            json!({
                "tier": "none",
                "status": status_code,
                "candidates": [],
                "reason": err_msg
            }),
        ),
    }
}

fn handle_candidates(
    service: &DailyNewsService,
    method: &str,
    body: &Value,
    limit: usize,
) -> Value {
    if method == "POST" && body.get("items").is_some() {
        let raw_items: Vec<RawNewsItem> =
            serde_json::from_value(body["items"].clone()).unwrap_or_default();
        let (from_date, to_date) = extract_date_range(body);
        let from_epoch = from_date
            .as_deref()
            .and_then(|d| daily_news::date::parse_boundary_datetime(d, false).ok());
        let to_epoch = to_date
            .as_deref()
            .and_then(|d| daily_news::date::parse_boundary_datetime(d, true).ok());
        let topics = extract_topics(body);

        let ranked =
            service.filter_and_score(&raw_items, from_epoch, to_epoch, topics.as_deref(), limit);
        let count = ranked.len();
        return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
    }

    let ranked = service.get_candidates(limit);
    let count = ranked.len();
    make_json_resp(200, json!({ "count": count, "candidates": ranked }))
}

fn handle_feed(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
    let sources = extract_sources(body);
    let mut feed = if let Some(ref s) = sources {
        service.get_feed_sources(s, limit)
    } else {
        service.get_feed(limit)
    };
    feed.truncate(limit);
    let count = feed.len();
    make_json_resp(200, json!({ "count": count, "items": feed }))
}

fn handle_consume(service: &DailyNewsService, body: &Value) -> Value {
    let ids: Vec<String> = if let Some(arr) = body["candidate_ids"].as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    } else if let Some(single) = body["candidate_id"].as_str() {
        vec![single.to_string()]
    } else {
        Vec::new()
    };

    match service.consume(&ids) {
        Ok(count) => make_json_resp(200, json!({ "status": "consumed", "count": count })),
        Err(e) => make_json_resp(500, json!({ "error": e })),
    }
}

fn handle_discussion(service: &DailyNewsService, body: &Value) -> Value {
    let url_or_id = body["url"]
        .as_str()
        .or_else(|| body["discussion_url"].as_str())
        .or_else(|| body["id"].as_str())
        .unwrap_or("");

    if url_or_id.trim().is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'url', 'discussion_url', or 'id'" }),
        );
    }

    let limit = body["limit"].as_u64().map(|v| v as usize);
    match service.get_discussion(url_or_id, limit) {
        Ok(thread) => make_json_resp(200, json!(thread)),
        Err(e) => make_json_resp(404, json!({ "error": e })),
    }
}

fn handle_config(service: &DailyNewsService, method: &str, body: &Value) -> Value {
    if method == "POST" {
        let mut cfg = service.get_config();
        cfg.apply_updates(body);
        return match service.update_config(cfg) {
            Ok(updated) => make_json_resp(200, json!({ "status": "updated", "config": updated })),
            Err(e) => make_json_resp(500, json!({ "error": e })),
        };
    }
    let current = service.get_config();
    make_json_resp(200, json!(current))
}

fn handle_request(req: Value, service: &DailyNewsService) -> Value {
    let raw_path = req["path"].as_str().unwrap_or("");
    let (path, query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (raw_path.trim_end_matches('/'), ""),
    };

    let method = req["method"].as_str().unwrap_or("GET").to_ascii_uppercase();

    let body = parse_request_body(&req, query);
    let limit = body["limit"]
        .as_u64()
        .or_else(|| body["count"].as_u64())
        .unwrap_or(5) as usize;

    if path.ends_with("/health") {
        return handle_health(service);
    }
    if path.ends_with("/models") {
        return handle_models();
    }
    if path.ends_with("/ui") || path.ends_with("/custom/daily-news") {
        return handle_ui(service, limit);
    }
    if path.ends_with("/chat/completions") {
        return handle_chat_completions(service, limit);
    }
    if path.ends_with("/api/v1/discover") {
        return handle_discover(service, &body, limit);
    }
    if path.ends_with("/api/v1/candidates") {
        return handle_candidates(service, &method, &body, limit);
    }
    if path.ends_with("/api/v1/feed") {
        return handle_feed(service, &body, limit);
    }
    if path.ends_with("/api/v1/discussion") {
        return handle_discussion(service, &body);
    }
    if path.ends_with("/api/v1/consume") {
        return handle_consume(service, &body);
    }
    if path.ends_with("/api/v1/config") {
        return handle_config(service, &method, &body);
    }

    make_json_resp(
        404,
        json!({ "error": format!("Route not found: {raw_path}") }),
    )
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

    let reddit_provider = if initial_config.reddit_enabled {
        Some(
            daily_news::reddit::RedditFeedProvider::new(initial_config.reddit_subreddits.clone())
                .with_credentials(
                    initial_config.reddit_client_id.clone(),
                    initial_config.reddit_client_secret.clone(),
                )
                .with_base_url(initial_config.reddit_base_url.clone()),
        )
    } else {
        None
    };

    let mut fetcher = LiveFeedFetcher::new(Some(initial_config.newsnow_url));
    if let Some(rp) = redis_provider {
        fetcher = fetcher.with_redis(rp);
    }
    if let Some(rp) = reddit_provider {
        fetcher = fetcher.with_reddit(rp);
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
                let err_resp =
                    make_json_resp(400, json!({ "error": format!("Invalid JSON: {e}") }));
                println!("{}", serde_json::to_string(&err_resp).unwrap_or_default());
                continue;
            }
        };

        let resp = handle_request(req, &service);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}
