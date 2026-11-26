//! Traffic Statistics & Savings HTTP/JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.

use crate::ports::MetricsPort;
use crate::rpc::make_json_resp;
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_get_metrics(service: &ProxyGuardService) -> Value {
    let metrics = service.get_metrics();
    let total_bytes = metrics.bytes_saved.saturating_add(metrics.upstream_bytes_used);
    let gb_saved = (metrics.bytes_saved as f64 / 1_073_741_824.0 * 10_000.0).round() / 10_000.0;
    let gb_upstream_used =
        (metrics.upstream_bytes_used as f64 / 1_073_741_824.0 * 10_000.0).round() / 10_000.0;
    let gb_total_original =
        (total_bytes as f64 / 1_073_741_824.0 * 10_000.0).round() / 10_000.0;
    let data_reduction_percentage = if total_bytes > 0 {
        ((metrics.bytes_saved as f64 / total_bytes as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };
    let mut val = json!(metrics);
    if let Some(obj) = val.as_object_mut() {
        obj.insert("gb_saved".to_string(), json!(gb_saved));
        obj.insert("gb_upstream_used".to_string(), json!(gb_upstream_used));
        obj.insert("gb_total_original".to_string(), json!(gb_total_original));
        obj.insert(
            "data_reduction_percentage".to_string(),
            json!(data_reduction_percentage),
        );
    }
    make_json_resp(200, val)
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
