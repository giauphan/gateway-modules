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
