//! Integration tests for Upstream Multi-Proxy Pool, rotation, failover, and API/MCP.

use proxy_guard::domain::{
    ProxyProtocol, ProxyRotationStrategy, ProxySelectionQuery, UpstreamProxy,
};
use proxy_guard::handlers::handle_request;
use proxy_guard::ports::ProxyPoolManagementPort;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use serde_json::json;
use std::sync::Arc;

fn make_service_with_pool(temp_dir: &std::path::Path) -> ProxyGuardService {
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
fn test_proxy_parsing_formats() {
    // 1. URL with auth
    let p1 = UpstreamProxy::parse(
        "http://alice:secret123@10.0.0.1:8080",
        Some("proxy-1"),
        Some("US"),
        Some(vec!["res".to_string()]),
    )
    .unwrap();
    assert_eq!(p1.protocol, ProxyProtocol::Http);
    assert_eq!(p1.host, "10.0.0.1");
    assert_eq!(p1.port, 8080);
    assert_eq!(p1.country.as_deref(), Some("US"));
    assert_eq!(p1.auth.as_ref().unwrap().username, "alice");
    assert_eq!(p1.auth.as_ref().unwrap().password, "secret123");

    // 2. SOCKS5 URL
    let p2 = UpstreamProxy::parse("socks5://127.0.0.1:1080", None, None, None).unwrap();
    assert_eq!(p2.protocol, ProxyProtocol::Socks5);
    assert_eq!(p2.port, 1080);
    assert!(p2.auth.is_none());

    // 3. Vendor format host:port:user:pass
    let p3 = UpstreamProxy::parse(
        "192.168.1.50:9000:myuser:mypass",
        Some("vendor-proxy"),
        Some("VN"),
        None,
    )
    .unwrap();
    assert_eq!(p3.host, "192.168.1.50");
    assert_eq!(p3.port, 9000);

    // 4. Bracketed IPv6 format
    let p4 = UpstreamProxy::parse("http://[2001:db8::1]:8080", None, None, None).unwrap();
    assert_eq!(p4.host, "[2001:db8::1]");
    assert_eq!(p4.port, 8080);

    // 5. Credentials with percent-encoding, reserved chars, multibyte UTF-8 and literal '+'
    let p5 = UpstreamProxy::parse(
        "http://user%201:pass%2Bword%2F123%C3%A9@[2001:db8::1]:9090",
        None,
        None,
        None,
    )
    .unwrap();
    let auth = p5.auth.as_ref().unwrap();
    assert_eq!(auth.username, "user 1");
    assert_eq!(auth.password, "pass+word/123é");
    // Canonical URL percent-encodes userinfo so authority parsing remains valid
    assert!(p5.url.contains("pass%2Bword%2F123%C3%A9"));
    assert_eq!(p3.country.as_deref(), Some("VN"));
    assert_eq!(p3.auth.as_ref().unwrap().username, "myuser");

    // 4. Playwright config conversion
    let pw_cfg = p1.to_playwright_config();
    assert_eq!(pw_cfg.server, "http://10.0.0.1:8080");
    assert_eq!(pw_cfg.username.as_deref(), Some("alice"));
    assert_eq!(pw_cfg.password.as_deref(), Some("secret123"));
}

#[test]
fn test_round_robin_rotation() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_rr_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    service
        .add_proxy(UpstreamProxy::parse("http://1.1.1.1:8080", Some("p1"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://2.2.2.2:8080", Some("p2"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://3.3.3.3:8080", Some("p3"), None, None).unwrap())
        .unwrap();

    let q = ProxySelectionQuery::default();
    let sel1 = service.select_proxy(&q).unwrap();
    let sel2 = service.select_proxy(&q).unwrap();
    let sel3 = service.select_proxy(&q).unwrap();
    let sel4 = service.select_proxy(&q).unwrap();

    // Verify round-robin sequence
    assert_ne!(sel1.id, sel2.id);
    assert_ne!(sel2.id, sel3.id);
    assert_eq!(sel1.id, sel4.id);
}

#[test]
fn test_sticky_session_rotation() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_sticky_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);
    service
        .set_strategy(ProxyRotationStrategy::StickySession)
        .unwrap();

    service
        .add_proxy(UpstreamProxy::parse("http://10.0.0.1:8080", Some("p1"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://10.0.0.2:8080", Some("p2"), None, None).unwrap())
        .unwrap();

    let q_session_a = ProxySelectionQuery {
        session_id: Some("session-alpha".to_string()),
        ..Default::default()
    };
    let q_session_b = ProxySelectionQuery {
        session_id: Some("session-beta".to_string()),
        ..Default::default()
    };

    let p_a1 = service.select_proxy(&q_session_a).unwrap();
    let p_a2 = service.select_proxy(&q_session_a).unwrap();
    let p_a3 = service.select_proxy(&q_session_a).unwrap();

    // Session A should always receive the same sticky proxy
    assert_eq!(p_a1.id, p_a2.id);
    assert_eq!(p_a2.id, p_a3.id);

    // Session B selects a proxy
    let p_b1 = service.select_proxy(&q_session_b).unwrap();
    let p_b2 = service.select_proxy(&q_session_b).unwrap();
    assert_eq!(p_b1.id, p_b2.id);
}

#[test]
fn test_country_and_tag_filtering() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_filter_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    service
        .add_proxy(
            UpstreamProxy::parse(
                "http://1.1.1.1:8080",
                Some("us-res"),
                Some("US"),
                Some(vec!["residential".to_string()]),
            )
            .unwrap(),
        )
        .unwrap();
    service
        .add_proxy(
            UpstreamProxy::parse(
                "http://2.2.2.2:8080",
                Some("vn-dc"),
                Some("VN"),
                Some(vec!["datacenter".to_string()]),
            )
            .unwrap(),
        )
        .unwrap();

    // Query US
    let q_us = ProxySelectionQuery {
        country: Some("US".to_string()),
        ..Default::default()
    };
    let selected_us = service.select_proxy(&q_us).unwrap();
    assert_eq!(selected_us.country.as_deref(), Some("US"));

    // Query Datacenter tag
    let q_tag = ProxySelectionQuery {
        tag: Some("datacenter".to_string()),
        ..Default::default()
    };
    let selected_tag = service.select_proxy(&q_tag).unwrap();
    assert_eq!(selected_tag.name, "vn-dc");
}

#[test]
fn test_automatic_fallback_selection() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_fb_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    service
        .add_proxy(
            UpstreamProxy::parse("http://1.1.1.1:8080", Some("p1"), Some("US"), None).unwrap(),
        )
        .unwrap();
    service
        .add_proxy(
            UpstreamProxy::parse("http://2.2.2.2:8080", Some("p2"), Some("US"), None).unwrap(),
        )
        .unwrap();

    let q = ProxySelectionQuery {
        country: Some("US".to_string()),
        ..Default::default()
    };

    // Primary proxy fails
    let failed_id = "1.1.1.1:8080";
    service.record_proxy_result(failed_id, false);

    let fallback = service.select_fallback(failed_id, &q).unwrap();
    assert_eq!(fallback.id, "2.2.2.2:8080");
}

#[test]
fn test_json_rpc_proxy_endpoints() {
    let temp_dir = std::env::temp_dir().join(format!("pg_rpc_pool_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    // 1. Batch add proxies via JSON-RPC POST /proxies/add
    let req_add = json!({
        "method": "POST",
        "path": "/proxies/add",
        "body": {
            "proxies": [
                { "url": "http://userA:passA@10.10.10.1:8000", "country": "US", "tags": ["res"] },
                { "url": "http://userB:passB@10.10.10.2:8000", "country": "SG", "tags": ["dc"] }
            ]
        }
    });
    let resp_add = handle_request(req_add, &service);
    assert_eq!(resp_add["status"], 200);

    // 2. Query proxies list
    let req_list = json!({
        "method": "GET",
        "path": "/proxies/list"
    });
    let resp_list = handle_request(req_list, &service);
    assert_eq!(resp_list["status"], 200);
    let body: serde_json::Value =
        serde_json::from_str(resp_list["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["total_proxies"], 2);

    // 3. Query next proxy for tool
    let req_next = json!({
        "method": "POST",
        "path": "/proxies/next",
        "body": {
            "country": "SG"
        }
    });
    let resp_next = handle_request(req_next, &service);
    assert_eq!(resp_next["status"], 200);
    let next_body: serde_json::Value =
        serde_json::from_str(resp_next["body"].as_str().unwrap()).unwrap();
    assert_eq!(next_body["proxy"]["country"], "SG");
    assert!(next_body["playwright_config"]["server"]
        .as_str()
        .unwrap()
        .contains("10.10.10.2:8000"));
}

#[test]
fn test_mcp_proxy_tools() {
    let temp_dir = std::env::temp_dir().join(format!("pg_mcp_pool_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    // MCP: proxy-guard__add_upstream_proxy
    let req_add = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__add_upstream_proxy",
            "arguments": {
                "url": "http://res_user:res_pass@residential.pool.net:9999",
                "name": "premium-res",
                "country": "US"
            }
        }
    });
    let resp_add = handle_request(req_add, &service);
    assert_eq!(resp_add["status"], 200);

    // MCP: proxy-guard__get_next_proxy
    let req_get = json!({
        "method": "POST",
        "path": "/tools/call",
        "body": {
            "name": "proxy-guard__get_next_proxy",
            "arguments": {}
        }
    });
    let resp_get = handle_request(req_get, &service);
    assert_eq!(resp_get["status"], 200);
    let mcp_res: serde_json::Value =
        serde_json::from_str(resp_get["body"].as_str().unwrap()).unwrap();
    assert_eq!(mcp_res["isError"], false);
}
