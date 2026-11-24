use proxy_guard::handlers::handle_request;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use serde_json::json;
use std::sync::Arc;

#[test]
fn test_rpc_health_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("pg_rpc_test_{}", std::process::id()));
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let service = ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        3.5,
    );

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
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let service = ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        3.5,
    );

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
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let service = ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        3.5,
    );

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
