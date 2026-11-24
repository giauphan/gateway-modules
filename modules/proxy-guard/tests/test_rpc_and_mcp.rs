use proxy_guard::domain::ProxyRotationStrategy;
use proxy_guard::handlers::handle_request;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use serde_json::{json, Value};
use std::sync::Arc;

fn make_test_service(temp_dir: &std::path::Path) -> ProxyGuardService {
    let storage = Arc::new(FileStorageAdapter::new(temp_dir));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    )
}

#[test]
fn test_rpc_health_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("pg_rpc_test_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    let req = json!({
        "method": "GET",
        "path": "/custom/proxy-guard/health"
    });

    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("healthy"));
    assert!(body_str.contains("proxy-guard"));
}

#[test]
fn test_rpc_evaluate_url() {
    let temp_dir = std::env::temp_dir().join(format!("pg_rpc_eval_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    let req = json!({
        "method": "POST",
        "path": "/custom/proxy-guard/evaluate",
        "body": {
            "url": "https://www.google-analytics.com/analytics.js",
            "method": "GET",
            "resource_type": "script"
        }
    });

    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("mock_script"));
}

#[test]
fn test_mcp_tool_invocation() {
    let temp_dir = std::env::temp_dir().join(format!("pg_mcp_test_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    let req = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__evaluate_url",
            "arguments": {
                "url": "https://connect.facebook.net/en_US/fbevents.js",
                "method": "GET"
            }
        }
    });

    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("BLOCKED"));
    assert!(body_str.contains("fbevents.js"));
}

#[test]
fn test_mcp_optimize_flow_har_and_validation() {
    let temp_dir = std::env::temp_dir().join(format!("pg_mcp_har_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. HAR format with nested request.url and response.bodySize
    let req = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__optimize_flow",
            "arguments": {
                "requests": [
                    {
                        "request": {
                            "url": "https://www.google-analytics.com/analytics.js",
                            "method": "GET"
                        },
                        "response": {
                            "bodySize": 52000
                        }
                    },
                    {
                        "url": "https://example.com/api/products",
                        "method": "GET",
                        "size_bytes": 1000
                    }
                ]
            }
        }
    });

    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("flow_summary"));
    assert!(body_str.contains("blocked_requests"));
    assert!(body_str.contains("forwarded_requests"));

    // 2. Validation error for entry missing both forms
    let invalid_req = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__optimize_flow",
            "arguments": {
                "requests": [
                    { "invalid_key": 123 }
                ]
            }
        }
    });
    let invalid_resp = handle_request(invalid_req, &service);
    let invalid_body = invalid_resp["body"].as_str().unwrap();
    assert!(invalid_body.contains("Invalid request entry"));
}

#[test]
fn test_models_endpoint_and_mcp() {
    let temp_dir = std::env::temp_dir().join(format!("pg_models_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. Direct /v1/models route
    let req = json!({
        "method": "GET",
        "path": "/v1/models"
    });
    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("proxy-guard"));
    assert!(body_str.contains("model"));

    // 2. MCP tool proxy-guard__list_models
    let mcp_req = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__list_models",
            "arguments": {}
        }
    });
    let mcp_resp = handle_request(mcp_req, &service);
    assert_eq!(mcp_resp["status"], 200);
    let mcp_body = mcp_resp["body"].as_str().unwrap();
    assert!(mcp_body.contains("proxy-guard"));
}

#[test]
fn test_chat_completions_endpoint_and_url_evaluation() {
    let temp_dir = std::env::temp_dir().join(format!("pg_chat_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. Chat completion asking for status / bandwidth overview
    let status_req = json!({
        "method": "POST",
        "path": "/v1/chat/completions",
        "body": {
            "messages": [
                { "role": "user", "content": "How much bandwidth have we saved?" }
            ]
        }
    });
    let status_resp = handle_request(status_req, &service);
    assert_eq!(status_resp["status"], 200);
    let status_body = status_resp["body"].as_str().unwrap();
    assert!(status_body.contains("chat.completion"));
    assert!(status_body.contains("Proxy Guard"));
    assert!(status_body.contains("Bandwidth Saved"));

    // 2. Chat completion asking to evaluate a URL
    let eval_req = json!({
        "method": "POST",
        "path": "/chat/completions",
        "body": {
            "messages": [
                { "role": "user", "content": "Can you check https://www.google-analytics.com/analytics.js please?" }
            ]
        }
    });
    let eval_resp = handle_request(eval_req, &service);
    assert_eq!(eval_resp["status"], 200);
    let eval_body = eval_resp["body"].as_str().unwrap();
    assert!(eval_body.contains("BLOCKED"));
    assert!(eval_body.contains("google-analytics.com"));

    // 3. MCP tool proxy-guard__chat
    let mcp_chat = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__chat",
            "arguments": {
                "prompt": "Evaluate https://quotes.toscrape.com"
            }
        }
    });
    let mcp_chat_resp = handle_request(mcp_chat, &service);
    assert_eq!(mcp_chat_resp["status"], 200);
    let mcp_chat_body = mcp_chat_resp["body"].as_str().unwrap();
    assert!(mcp_chat_body.contains("FORWARDED"));
}

#[test]
fn test_tools_list_and_header_dispatch() {
    let temp_dir = std::env::temp_dir().join(format!("pg_tools_list_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. /tools/list endpoint
    let req = json!({
        "method": "GET",
        "path": "/tools/list"
    });
    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    let tools = body["tools"].as_array().expect("Tools must be an array");
    assert!(tools.len() >= 6, "Must expose all registered MCP tools");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(tool_names.contains(&"proxy-guard__evaluate_url"));
    assert!(tool_names.contains(&"proxy-guard__optimize_flow"));
    assert!(tool_names.contains(&"proxy-guard__chat"));
    assert!(tool_names.contains(&"proxy-guard__list_models"));

    // 2. Dispatch via x-mcp-tool header
    let header_req = json!({
        "method": "POST",
        "path": "/any-path",
        "headers": {
            "x-mcp-tool": "proxy-guard__list_models"
        },
        "body": {}
    });
    let header_resp = handle_request(header_req, &service);
    assert_eq!(header_resp["status"], 200);
    let header_body = header_resp["body"].as_str().unwrap();
    assert!(header_body.contains("proxy-guard"));
}

#[test]
fn test_not_found_and_bad_request_routes() {
    let temp_dir = std::env::temp_dir().join(format!("pg_err_routes_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. 404 Not Found
    let nf_req = json!({
        "method": "GET",
        "path": "/nonexistent/endpoint/404"
    });
    let nf_resp = handle_request(nf_req, &service);
    assert_eq!(nf_resp["status"], 404);
    assert!(nf_resp["body"].as_str().unwrap().contains("Not Found"));

    // 2. 400 Bad Request on invalid body format
    let bad_req = json!({
        "method": "POST",
        "path": "/evaluate",
        "body": "this is not valid json or object for evaluate"
    });
    let bad_resp = handle_request(bad_req, &service);
    assert_eq!(bad_resp["status"], 400);
}
