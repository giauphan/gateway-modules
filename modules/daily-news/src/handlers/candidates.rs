//! Candidate discovery, feed aggregation, capture, enrichment, evidence, and discussion handlers.

use crate::domain::RawNewsItem;
use crate::rpc::{extract_date_range, extract_sources, extract_topics, make_json_resp};
use crate::service::DailyNewsService;
use serde_json::{json, Value};
use std::path::Path;

pub fn handle_chat_completions(service: &DailyNewsService, limit: usize) -> Value {
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

pub fn handle_discover(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
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

pub fn handle_candidates(
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
            .and_then(|d| crate::date::parse_boundary_datetime(d, false).ok());
        let to_epoch = to_date
            .as_deref()
            .and_then(|d| crate::date::parse_boundary_datetime(d, true).ok());
        let topics = extract_topics(body);

        let ranked =
            service.filter_and_score(&raw_items, from_epoch, to_epoch, topics.as_deref(), limit);
        let count = ranked.len();
        return make_json_resp(200, json!({ "count": count, "candidates": ranked }));
    }

    let ranked = match service.try_get_candidates(limit) {
        Ok(r) => r,
        Err(e) => {
            return make_json_resp(
                500,
                json!({ "error": format!("Persistence error during candidate discovery: {e}") }),
            );
        }
    };
    let count = ranked.len();
    make_json_resp(200, json!({ "count": count, "candidates": ranked }))
}

pub fn handle_feed(service: &DailyNewsService, body: &Value, limit: usize) -> Value {
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

pub fn handle_consume(service: &DailyNewsService, body: &Value) -> Value {
    let ids: Vec<String> = body["candidate_ids"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string))
                .collect()
        })
        .or_else(|| {
            body["candidate_id"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| vec![s.to_string()])
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing or empty 'candidate_id' or 'candidate_ids' parameter" }),
        );
    }

    match service.consume(&ids) {
        Ok(count) => make_json_resp(200, json!({ "status": "consumed", "count": count })),
        Err(e) => make_json_resp(500, json!({ "error": e })),
    }
}

pub fn handle_discussion(service: &DailyNewsService, body: &Value) -> Value {
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

pub fn handle_capture(service: &DailyNewsService, body: &Value) -> Value {
    let extract_url = |k: &str| body[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    let url = extract_url("url")
        .or_else(|| extract_url("source_url"))
        .or_else(|| extract_url("canonical_url"))
        .unwrap_or("");
    if url.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'url', 'source_url', or 'canonical_url'" }),
        );
    }
    match service.capture_primary_source(url) {
        Ok(evidence) => make_json_resp(200, json!(evidence)),
        Err(e) => make_json_resp(400, json!({ "error": e })),
    }
}

pub fn handle_enrich_candidate(
    service: &DailyNewsService,
    candidate_id: &str,
    body: &Value,
) -> Value {
    let override_url = body["override_url"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["url"].as_str().filter(|s| !s.trim().is_empty()));

    let claims = match body.get("claims").filter(|v| !v.is_null()) {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(c) => Some(c),
            Err(e) => {
                return make_json_resp(400, json!({ "error": format!("Invalid 'claims': {e}") }))
            }
        },
        None => None,
    };

    let visuals_val = body
        .get("visual_evidence")
        .filter(|v| !v.is_null())
        .or_else(|| body.get("visuals").filter(|v| !v.is_null()));

    let visuals = match visuals_val {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(vis) => Some(vis),
            Err(e) => {
                return make_json_resp(
                    400,
                    json!({ "error": format!("Invalid 'visual_evidence': {e}") }),
                )
            }
        },
        None => None,
    };

    match service.enrich_candidate(candidate_id, override_url, claims, visuals) {
        Ok(resp) => make_json_resp(200, json!(resp)),
        Err(e) => make_json_resp(400, json!({ "error": e })),
    }
}

pub fn handle_enrich(service: &DailyNewsService, body: &Value) -> Value {
    if let Some(cid_val) = body.get("candidate_id") {
        let cid = cid_val.as_str().unwrap_or("").trim();
        if cid.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'candidate_id'" }),
            );
        }
        return handle_enrich_candidate(service, cid, body);
    }
    handle_capture(service, body)
}

pub fn handle_evidence(base_dir: &Path, body: &Value) -> Value {
    let sha256_raw = body["sha256"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["hash"].as_str().filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
        .trim();

    if sha256_raw.is_empty()
        || sha256_raw.len() != 64
        || !sha256_raw.chars().all(|c| c.is_ascii_hexdigit())
    {
        return make_json_resp(
            400,
            json!({ "error": "Invalid or missing sha256 hash parameter (expected 64 hex characters)" }),
        );
    }
    let sha256 = sha256_raw.to_ascii_lowercase();

    let state_dir = crate::storage::resolve_state_dir(base_dir);
    let evidence_path = state_dir
        .join("evidence")
        .join(format!("{sha256}.txt"));
    if !evidence_path.exists() {
        return make_json_resp(
            404,
            json!({ "error": format!("Evidence snapshot not found for sha256: {sha256}") }),
        );
    }

    match std::fs::read_to_string(&evidence_path) {
        Ok(content) => make_json_resp(
            200,
            json!({
                "sha256": sha256,
                "ref": format!(".state/evidence/{sha256}.txt"),
                "content": content
            }),
        ),
        Err(e) => make_json_resp(
            500,
            json!({ "error": format!("Failed to read snapshot file: {e}") }),
        ),
    }
}
