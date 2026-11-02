//! Standalone Native Rust stdio-stream entrypoint for Toolcase Gateway.

use daily_news::domain::{
    canonicalize_url, classify_topic_family, compute_sha256, DiscoveryResult, NormalizedCandidate,
};
use daily_news::scoring::CandidateScorer;
use serde_json::{json, Value};
use std::io::{self, BufRead};

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

fn fetch_live_hackernews(limit: usize) -> Vec<NormalizedCandidate> {
    let endpoint = "https://hacker-news.firebaseio.com/v0/topstories.json";
    let ids: Vec<u64> = match ureq::get(endpoint).call() {
        Ok(res) => res.into_body().read_json().unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let target_ids: Vec<u64> = ids.into_iter().take(limit.max(25)).collect();
    let mut candidates = Vec::new();

    for id in target_ids {
        let item_url = format!("https://hacker-news.firebaseio.com/v0/item/{id}.json");
        let val: Value = match ureq::get(&item_url).call() {
            Ok(res) => res.into_body().read_json().unwrap_or_default(),
            Err(_) => continue,
        };

        let title = val["title"].as_str().unwrap_or("").trim().to_string();
        let orig_url = val["url"]
            .as_str()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={id}"));
        if title.is_empty() {
            continue;
        }

        let canonical = canonicalize_url(&orig_url);
        let disc_url = format!("https://news.ycombinator.com/item?id={id}");
        let chash = compute_sha256(&format!("{title}:{canonical}"));
        let thash = compute_sha256(&title);
        let topic = classify_topic_family(&title, "");

        candidates.push(NormalizedCandidate {
            candidate_id: format!("cand_{}", &chash[..16]),
            canonical_url: canonical,
            original_url: orig_url,
            title,
            publisher: "Hacker News".to_string(),
            normalized_publisher: "Hacker News".to_string(),
            published_at_utc: chrono_fallback_now(),
            content_hash: chash,
            title_hash: thash,
            description: None,
            body: None,
            summary: None,
            discussion_url: Some(disc_url),
            topic_family: topic,
            tags: vec!["hackernews".to_string(), "tech".to_string()],
            scoring: None,
        });
    }
    candidates
}

fn chrono_fallback_now() -> String {
    "2026-11-02T11:00:00+00:00".to_string()
}

fn handle_request(req: Value) -> Value {
    let raw_path = req["path"].as_str().unwrap_or("");
    let (path, query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.trim_end_matches('/'), q),
        None => (raw_path.trim_end_matches('/'), ""),
    };

    let mut body: Value = if let Some(b) = req["body"].as_str() {
        serde_json::from_str(b).unwrap_or_else(|_| json!({}))
    } else if req["body"].is_object() {
        req["body"].clone()
    } else {
        json!({})
    };

    // Extract query parameters
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
        return make_json_resp(
            200,
            json!({
                "status": "healthy",
                "module": "daily-news",
                "version": "1.3.0",
                "engine": "rust-native"
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
        return make_html_resp(200, "<html><head><title>Daily News</title></head><body><h1>Daily News & Content Discovery (Rust Native)</h1><p>Engine: Standalone Rust binary with stdio-stream protocol.</p></body></html>");
    }

    // 4. Chat Completions
    if path.ends_with("/chat/completions") {
        let scorer = CandidateScorer::default();
        let raw = fetch_live_hackernews(limit);
        let ranked = scorer.rank_candidates(raw, limit);

        let mut lines = vec![format!("### Daily News Briefing ({} items):\n", ranked.len())];
        for (i, c) in ranked.iter().enumerate() {
            let score = c.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
            let mut line = format!("{}. **{}** ({}) [Score: {:.2}]\n   {}", i + 1, c.title, c.publisher, score, c.canonical_url);
            if let Some(ref d) = c.discussion_url {
                if d != &c.canonical_url {
                    line.push_str(&format!("\n   Discussion: {}", d));
                }
            }
            lines.push(line);
        }
        let reply = lines.join("\n");

        return make_json_resp(
            200,
            json!({
                "id": format!("chatcmpl-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                "object": "chat.completion",
                "model": "daily-news",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": reply },
                    "finish_reason": "stop"
                }]
            }),
        );
    }

    // 5. Tiered Discovery
    if path.ends_with("/api/v1/discover") {
        let scorer = CandidateScorer::default();
        let raw = fetch_live_hackernews(limit);
        let ranked = scorer.rank_candidates(raw, limit);
        let count = ranked.len();

        let result = DiscoveryResult {
            tier: "tier_2_automated".to_string(),
            status: "ELIGIBLE".to_string(),
            count,
            candidates: ranked,
            source_file: Some("live_hackernews".to_string()),
            reason: None,
            diagnostics: json!({
                "tier_2_checked": true,
                "engine": "rust-native"
            }),
        };
        return make_json_resp(200, json!(result));
    }

    // 6. Candidates Endpoint
    if path.ends_with("/api/v1/candidates") {
        let scorer = CandidateScorer::default();
        let raw = fetch_live_hackernews(limit);
        let ranked = scorer.rank_candidates(raw, limit);
        let count = ranked.len();
        return make_json_resp(
            200,
            json!({
                "count": count,
                "candidates": ranked
            }),
        );
    }

    make_json_resp(404, json!({ "error": format!("Route not found: {raw_path}") }))
}

fn main() {
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

        let resp = handle_request(req);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}
