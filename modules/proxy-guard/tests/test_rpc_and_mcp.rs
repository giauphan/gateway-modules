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

#[test]
fn test_dashboard_ui_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("pg_ui_test_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    let req = json!({
        "method": "GET",
        "path": "/custom/proxy-guard/ui"
    });
    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body_str = resp["body"].as_str().unwrap();
    assert!(body_str.contains("Proxy Guard Dashboard"));
    assert!(body_str.contains("Data Optimized (GB Saved)"));
    assert!(body_str.contains("Data Optimization Efficiency"));
    assert!(body_str.contains("GB"));
    assert!(body_str.contains("Registered Upstream Proxies"));
    assert!(body_str.contains("Add Multiple Upstream Proxies"));
}

#[test]
fn test_config_endpoint_get_and_post() {
    let temp_dir = std::env::temp_dir().join(format!("pg_config_test_{}", std::process::id()));
    let service = make_test_service(&temp_dir);

    // 1. GET /modules/proxy-guard/config returns default JSON config
    let get_req = json!({
        "method": "GET",
        "path": "/modules/proxy-guard/config"
    });
    let get_resp = handle_request(get_req, &service);
    assert_eq!(get_resp["status"], 200);
    let body: Value = serde_json::from_str(get_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["rotation_strategy"], "round_robin");
    assert_eq!(body["block_analytics"], true);
    assert_eq!(body["block_ads"], true);

    // 2. POST /modules/proxy-guard/config updates multi-proxy pool and rotation strategy
    let post_req = json!({
        "method": "POST",
        "path": "/modules/proxy-guard/config",
        "body": {
            "upstream_proxy_pool": "http://user1:pass1@proxy1.com:8080\nhttp://user2:pass2@proxy2.com:8080",
            "rotation_strategy": "lowest_failures",
            "block_media": false,
            "cost_per_gb_usd": 4.2
        }
    });
    let post_resp = handle_request(post_req, &service);
    assert_eq!(post_resp["status"], 200);
    let post_body: Value = serde_json::from_str(post_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(post_body["status"], "success");
    assert_eq!(post_body["config"]["rotation_strategy"], "lowest_failures");
    assert_eq!(post_body["config"]["block_media"], false);
    assert_eq!(post_body["config"]["cost_per_gb_usd"], 4.2);
    assert!(post_body["config"]["upstream_proxy_pool"]
        .as_str()
        .unwrap()
        .contains("proxy1.com"));

    // 3. GET /modules/proxy-guard/config with text/html Accept header returns dashboard HTML
    let html_req = json!({
        "method": "GET",
        "path": "/modules/proxy-guard/config",
        "headers": {
            "accept": "text/html,application/xhtml+xml"
        }
    });
    let html_resp = handle_request(html_req, &service);
    assert_eq!(html_resp["status"], 200);
    assert!(html_resp["body"]
        .as_str()
        .unwrap()
        .contains("Proxy Guard Dashboard"));

    // 4. GET /config alias also works
    let alias_req = json!({
        "method": "GET",
        "path": "/config"
    });
    let alias_resp = handle_request(alias_req, &service);
    assert_eq!(alias_resp["status"], 200);
}

#[test]
fn test_request_logs_api_and_mcp() {
    use proxy_guard::ports::{FlowOptimizerPort, MetricsPort};
    let temp_dir = std::env::temp_dir().join(format!("pg_logs_test_{}", std::process::id()));
    let service = make_test_service(&temp_dir);
    service.reset_metrics();

    // 1. Initially empty logs
    let req_logs_empty = json!({
        "method": "GET",
        "path": "/custom/proxy-guard/logs"
    });
    let resp_empty = handle_request(req_logs_empty, &service);
    assert_eq!(resp_empty["status"], 200);
    let body_empty: Value = serde_json::from_str(resp_empty["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_empty["logs_count"], 0);
    assert_eq!(body_empty["request_reduction_percentage"], 0.0);

    // 2. Intercept a blocked request and an allowed request
    let _ = service.evaluate_url("https://www.google-analytics.com/analytics.js", "GET", None);
    let _ = service.evaluate_url("https://api.ipify.org?format=json", "GET", None);

    // 3. Query GET /custom/proxy-guard/logs
    let req_logs = json!({
        "method": "GET",
        "path": "/custom/proxy-guard/logs?limit=10"
    });
    let resp = handle_request(req_logs, &service);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["logs_count"], 2);
    assert_eq!(body["total_requests"], 2);
    assert_eq!(body["blocked_requests"], 1);
    assert_eq!(body["forwarded_requests"], 1);
    assert_eq!(body["request_reduction_percentage"], 50.0);

    let logs = body["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0]["host"], "api.ipify.org");
    assert_eq!(logs[0]["action"], "Forwarded (Upstream)");
    assert_eq!(logs[1]["host"], "www.google-analytics.com");
    assert!(logs[1]["action"].as_str().unwrap().contains("Mocked"));

    // 4. Query via MCP tool proxy-guard__get_request_logs
    let mcp_req = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__get_request_logs",
            "arguments": { "limit": 5 }
        }
    });
    let mcp_resp = handle_request(mcp_req, &service);
    assert_eq!(mcp_resp["status"], 200);
    let mcp_body: Value = serde_json::from_str(mcp_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_body["isError"], false);

    // 5. POST /custom/proxy-guard/logs/clear
    let clear_req = json!({
        "method": "POST",
        "path": "/custom/proxy-guard/logs/clear"
    });
    let clear_resp = handle_request(clear_req, &service);
    assert_eq!(clear_resp["status"], 200);
    assert_eq!(service.get_request_logs(10).len(), 0);
}
