//! Interface Adapters: OpenAI-Compatible Chat Completions & Models handlers.
//!
//! Clean Architecture: Interface Adapters Layer.
//! Allows Toolcase Gateway and external AI agents to interact with Proxy Guard
//! via standard /v1/chat/completions and /v1/models endpoints.

use crate::ports::{FlowOptimizerPort, MetricsPort, ProxyPoolManagementPort};
use crate::rpc::make_json_resp;
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_models() -> Value {
    make_json_resp(
        200,
        json!({
            "object": "list",
            "data": [
                {
                    "id": "proxy-guard",
                    "object": "model",
                    "created": 1700000000,
                    "owned_by": "proxy-guard-rs"
                }
            ]
        }),
    )
}

pub fn handle_chat_completions(body: &Value, service: &ProxyGuardService) -> Value {
    let last_user_message = body
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter().rev().find(|msg| {
                msg.get("role")
                    .and_then(|r| r.as_str())
                    .map(|r| r == "user")
                    .unwrap_or(false)
            })
        })
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str())
        .or_else(|| body.get("prompt").and_then(|p| p.as_str()))
        .or_else(|| body.get("message").and_then(|m| m.as_str()))
        .unwrap_or("");

    let reply = generate_assistant_response(last_user_message, service);
    let id = format!("chatcmpl-{}", uuid_short());

    let prompt_tokens = last_user_message.len().div_ceil(4);
    let completion_tokens = reply.len().div_ceil(4);
    let total_tokens = prompt_tokens + completion_tokens;

    make_json_resp(
        200,
        json!({
            "id": id,
            "object": "chat.completion",
            "created": 1700000000,
            "model": "proxy-guard",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": reply
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens
            }
        }),
    )
}

fn generate_assistant_response(user_input: &str, service: &ProxyGuardService) -> String {
    let trimmed = user_input.trim();

    // 1. Check if user provided a URL to evaluate
    if let Some(url) = extract_url(trimmed) {
        let decision = service.evaluate_url(url, "GET", None);
        if decision.is_blocked() {
            format!(
                "🛡️ **Proxy Guard Evaluation**: BLOCKED\n\n- **Target URL**: `{}`\n- **Action**: `{:?}`\n- **Reason**: {}\n- **Estimated Bandwidth Saved**: ~{} bytes\n\nThis request is intercepted locally by Proxy Guard with zero upstream proxy bandwidth consumption.",
                decision.url, decision.action, decision.reason, decision.estimated_bytes_saved
            )
        } else {
            format!(
                "✅ **Proxy Guard Evaluation**: FORWARDED\n\n- **Target URL**: `{}`\n- **Action**: Forward to Upstream Paid Proxy\n- **Reason**: Clean business/target traffic required for automation.\n- **Optimizations Applied**: Tracking query parameters stripped, fingerprinting headers removed, RFC 9110 compression enforced.",
                decision.url
            )
        }
    } else {
        // 2. Status & Bandwidth Savings overview
        let metrics = service.get_metrics();
        let proxies = service.list_proxies();
        let strategy = service.get_strategy();

        format!(
            "🛡️ **Proxy Guard & Traffic Optimizer v{}**\n\n\
            **Cumulative Performance & Cost Savings**:\n\
            - **Total Requests Processed**: {}\n\
            - **Blocked/Mocked (0-byte Upstream)**: {} ({:.1}%)\n\
            - **Forwarded to Paid Proxy**: {}\n\
            - **Bandwidth Saved**: {:.2} MB ({} bytes)\n\
            - **Estimated Cost Saved**: ${:.4} USD\n\n\
            **Active Upstream Proxy Pool**:\n\
            - **Registered Proxies**: {}\n\
            - **Current Rotation Strategy**: `{:?}`\n\n\
            *Ask me to evaluate any URL (e.g., `evaluate https://www.google-analytics.com/analytics.js`), or configure upstream proxies.*",
            env!("CARGO_PKG_VERSION"),
            metrics.total_requests,
            metrics.blocked_requests,
            if metrics.total_requests > 0 {
                (metrics.blocked_requests as f64 / metrics.total_requests as f64) * 100.0
            } else {
                0.0
            },
            metrics.forwarded_requests,
            metrics.bytes_saved as f64 / (1024.0 * 1024.0),
            metrics.bytes_saved,
            metrics.estimated_cost_saved_usd,
            proxies.len(),
            strategy
        )
    }
}

fn extract_url(text: &str) -> Option<&str> {
    for word in text.split_whitespace() {
        let clean =
            word.trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '\'' || c == '`');
        if clean.starts_with("http://") || clean.starts_with("https://") {
            return Some(clean);
        }
    }
    None
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let hex = format!("{nanos:x}");
    if hex.len() >= 12 {
        hex[..12].to_string()
    } else {
        hex
    }
}
