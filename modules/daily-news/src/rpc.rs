//! JSON-RPC protocol framing, response helpers, and request parameter extractors.
//!
//! Follows Clean Architecture Interface Adapters layer:
//! Decouples raw JSON transport framing from application use-cases.

use serde_json::{json, Value};

pub fn make_json_resp(status: u16, data: Value) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": "application/json",
            "x-module-engine": "rust-native"
        },
        "body": serde_json::to_string(&data).unwrap_or_default()
    })
}

pub fn make_html_resp(status: u16, html: &str) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": "text/html; charset=utf-8",
            "x-module-engine": "rust-native"
        },
        "body": html
    })
}

fn extract_string_list(body: &Value, plural_key: &str, singular_key: &str) -> Option<Vec<String>> {
    if let Some(arr) = body[plural_key]
        .as_array()
        .or_else(|| body[singular_key].as_array())
    {
        let items: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        if !items.is_empty() {
            return Some(items);
        }
    }
    body[plural_key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body[singular_key].as_str().filter(|s| !s.trim().is_empty()))
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

pub fn extract_topics(body: &Value) -> Option<Vec<String>> {
    extract_string_list(body, "topics", "topic")
}

pub fn extract_sources(body: &Value) -> Option<Vec<String>> {
    extract_string_list(body, "sources", "source")
}

pub fn extract_date_range(body: &Value) -> (Option<String>, Option<String>) {
    let from = body["from_date"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["since"].as_str().filter(|s| !s.trim().is_empty()))
        .or_else(|| body["start_date"].as_str().filter(|s| !s.trim().is_empty()))
        .or_else(|| body["from"].as_str().filter(|s| !s.trim().is_empty()))
        .map(str::to_string);

    let to = body["to_date"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| body["until"].as_str().filter(|s| !s.trim().is_empty()))
        .or_else(|| body["end_date"].as_str().filter(|s| !s.trim().is_empty()))
        .or_else(|| body["to"].as_str().filter(|s| !s.trim().is_empty()))
        .map(str::to_string);

    (from, to)
}

pub fn parse_request_body(req: &Value, query: &str) -> Value {
    let mut body: Value = if let Some(b) = req["body"].as_str() {
        serde_json::from_str(b).unwrap_or_else(|_| json!({}))
    } else if req["body"].is_object() {
        req["body"].clone()
    } else {
        json!({})
    };

    if !body.is_object() {
        body = json!({});
    }

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
