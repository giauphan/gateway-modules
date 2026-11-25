//! Traffic Statistics & Savings HTTP/JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.

use crate::ports::MetricsPort;
use crate::rpc::make_json_resp;
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_get_metrics(service: &ProxyGuardService) -> Value {
    let metrics = service.get_metrics();
    make_json_resp(200, json!(metrics))
}

pub fn handle_reset_metrics(service: &ProxyGuardService) -> Value {
    service.reset_metrics();
    make_json_resp(
        200,
        json!({ "status": "ok", "message": "Metrics reset to zero" }),
    )
}

pub fn handle_get_logs(query_val: &Value, service: &ProxyGuardService) -> Value {
    let limit = query_val
        .get("limit")
        .and_then(|v| {
            if let Some(n) = v.as_u64() {
                Some(n as usize)
            } else if let Some(s) = v.as_str() {
                s.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(50);

    let logs = service.get_request_logs(limit);
    let metrics = service.get_metrics();
    let total = metrics.total_requests;
    let blocked = metrics.blocked_requests;
    let reduction_pct = if total > 0 {
        (blocked as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    make_json_resp(
        200,
        json!({
            "status": "ok",
            "request_reduction_percentage": (reduction_pct * 10.0).round() / 10.0,
            "total_requests": total,
            "blocked_requests": blocked,
            "forwarded_requests": metrics.forwarded_requests,
            "bytes_saved": metrics.bytes_saved,
            "estimated_cost_saved_usd": metrics.estimated_cost_saved_usd,
            "logs_count": logs.len(),
            "logs": logs
        }),
    )
}

pub fn handle_clear_logs(service: &ProxyGuardService) -> Value {
    service.clear_request_logs();
    make_json_resp(
        200,
        json!({ "status": "ok", "message": "Request logs cleared" }),
    )
}
