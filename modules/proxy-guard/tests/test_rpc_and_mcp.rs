use proxy_guard::domain::ProxyRotationStrategy;
use proxy_guard::handlers::handle_request;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use serde_json::json;
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
