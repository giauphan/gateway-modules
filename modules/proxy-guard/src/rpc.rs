//! JSON-RPC protocol framing and response serialization helpers.
//!
//! Clean Architecture: Interface Adapters (Framing).

use serde_json::{json, Value};

pub fn make_json_resp(status: u16, data: Value) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": "application/json",
            "x-module-engine": "rust-native",
            "x-module-id": "proxy-guard"
        },
        "body": serde_json::to_string(&data).unwrap_or_default()
    })
}

pub fn make_text_resp(status: u16, content_type: &str, text: &str) -> Value {
    json!({
        "status": status,
        "headers": {
            "content-type": content_type,
            "x-module-engine": "rust-native",
            "x-module-id": "proxy-guard"
        },
        "body": text
    })
}

pub fn make_error_resp(status: u16, message: &str) -> Value {
    make_json_resp(status, json!({ "error": message }))
}

pub fn extract_request_body(req: &Value) -> Result<Value, String> {
    if let Some(body_val) = req.get("body") {
        if body_val.is_object() || body_val.is_array() {
            return Ok(body_val.clone());
        }
        if let Some(s) = body_val.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(json!({}));
            }
            return serde_json::from_str(trimmed)
                .map_err(|e| format!("Failed to parse request JSON body: {e}"));
        }
    }
    Ok(json!({}))
}
