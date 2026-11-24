//! Evaluation & Flow Analysis HTTP/JSON-RPC Handlers.
//!
//! Clean Architecture: Interface Adapters.

use crate::domain::FlowRequestDto;
use crate::ports::FlowOptimizerPort;
use crate::rpc::{make_error_resp, make_json_resp};
use crate::service::ProxyGuardService;
use serde_json::{json, Value};

pub fn handle_evaluate_url(body: &Value, service: &ProxyGuardService) -> Value {
    let url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return make_error_resp(400, "Missing required parameter 'url'"),
    };
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
    let resource_type = body.get("resource_type").and_then(|v| v.as_str());

    let decision = service.evaluate_url(url, method, resource_type);
    make_json_resp(200, json!(decision))
}

pub fn handle_optimize_flow(body: &Value, service: &ProxyGuardService) -> Value {
    let requests_val = match body.get("requests").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return make_error_resp(400, "Missing required array parameter 'requests'"),
    };

    let mut req_dtos = Vec::with_capacity(requests_val.len());
    for item in requests_val {
        if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
            req_dtos.push(FlowRequestDto {
                url: url.to_string(),
                method: item
                    .get("method")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                resource_type: item
                    .get("resource_type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                size_bytes: item.get("size_bytes").and_then(|v| v.as_u64()),
            });
        }
    }

    let result = service.optimize_flow(&req_dtos);
    make_json_resp(200, json!(result))
}
