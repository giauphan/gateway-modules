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

#[test]
fn test_lowest_failures_rotation_strategy() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_lf_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    service
        .set_strategy(ProxyRotationStrategy::LowestFailures)
        .unwrap();
    assert_eq!(
        service.get_strategy(),
        ProxyRotationStrategy::LowestFailures
    );

    service
        .add_proxy(UpstreamProxy::parse("http://1.1.1.1:8080", Some("p1"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://2.2.2.2:8080", Some("p2"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://3.3.3.3:8080", Some("p3"), None, None).unwrap())
        .unwrap();

    // Record failures for p1 and p2
    service.record_proxy_result("1.1.1.1:8080", false);
    service.record_proxy_result("1.1.1.1:8080", false);
    service.record_proxy_result("2.2.2.2:8080", false);
    // p3 has 0 failures

    let q = ProxySelectionQuery::default();
    let sel = service.select_proxy(&q).unwrap();
    assert_eq!(
        sel.id, "3.3.3.3:8080",
        "Lowest failures proxy must be selected"
    );

    // Fail p3 twice, p2 now has lowest failures (1 failure vs 2)
    service.record_proxy_result("3.3.3.3:8080", false);
    service.record_proxy_result("3.3.3.3:8080", false);
    let sel2 = service.select_proxy(&q).unwrap();
    assert_eq!(sel2.id, "2.2.2.2:8080");
}

#[test]
fn test_proxy_removal_and_invalid_inputs() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_remove_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    service
        .add_proxy(UpstreamProxy::parse("http://10.0.0.1:8080", Some("p1"), None, None).unwrap())
        .unwrap();
    assert_eq!(service.list_proxies().len(), 1);

    // Remove existing
    assert!(service.remove_proxy("10.0.0.1:8080").is_ok());
    assert_eq!(service.list_proxies().len(), 0);

    // Remove non-existent returns Err
    assert!(service.remove_proxy("10.0.0.1:8080").is_err());

    // Invalid proxy formats return Err
    assert!(UpstreamProxy::parse("", None, None, None).is_err());
    assert!(UpstreamProxy::parse("invalid_no_port", None, None, None).is_err());
    assert!(UpstreamProxy::parse("http://host:not_a_port", None, None, None).is_err());
    assert!(UpstreamProxy::parse("10.0.0.1:99999", None, None, None).is_err());
    assert!(UpstreamProxy::parse("[::1", None, None, None).is_err());
}

#[test]
fn test_per_request_rotation_strategy_override() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_per_req_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    // Global strategy is RoundRobin
    service
        .set_strategy(ProxyRotationStrategy::RoundRobin)
        .unwrap();

    service
        .add_proxy(UpstreamProxy::parse("http://1.1.1.1:8080", Some("p1"), None, None).unwrap())
        .unwrap();
    service
        .add_proxy(UpstreamProxy::parse("http://2.2.2.2:8080", Some("p2"), None, None).unwrap())
        .unwrap();

    // Mark p1 with failures
    service.record_proxy_result("1.1.1.1:8080", false);
    service.record_proxy_result("1.1.1.1:8080", false);

    // 1. Without per-request override: uses global RoundRobin (first round selects p1)
    let q_default = ProxySelectionQuery::default();
    let p_rr = service.select_proxy(&q_default).unwrap();
    assert_eq!(p_rr.id, "1.1.1.1:8080");

    // 2. With per-request override to LowestFailures: overrides RoundRobin and picks p2
    let q_override = ProxySelectionQuery {
        strategy: Some(ProxyRotationStrategy::LowestFailures),
        ..Default::default()
    };
    let p_lf = service.select_proxy(&q_override).unwrap();
    assert_eq!(
        p_lf.id, "2.2.2.2:8080",
        "Per-request strategy override must take precedence"
    );

    // 3. Header extraction test for X-Proxy-Strategy and X-Proxy-Session-Id
    let raw_headers = "Host: example.com\r\nX-Proxy-Strategy: lowest_failures\r\nX-Proxy-Session-Id: test-worker-1\r\n";
    let q_headers = proxy_guard::proxy_server::utils::extract_proxy_query_from_headers(raw_headers);
    assert_eq!(
        q_headers.strategy,
        Some(ProxyRotationStrategy::LowestFailures)
    );
    assert_eq!(q_headers.session_id.as_deref(), Some("test-worker-1"));

    // 4. API GET /proxies/next with query parameter ?strategy=lowest_failures
    let req_get = json!({
        "method": "GET",
        "path": "/proxies/next?strategy=lowest_failures"
    });
    let resp_get = handle_request(req_get, &service);
    assert_eq!(resp_get["status"], 200);
    let get_body: serde_json::Value =
        serde_json::from_str(resp_get["body"].as_str().unwrap()).unwrap();
    assert_eq!(get_body["strategy_applied"], "lowest_failures");
    assert_eq!(get_body["proxy"]["id"], "2.2.2.2:8080");

    // 5. API POST /proxies/next with body strategy
    let req_post = json!({
        "method": "POST",
        "path": "/proxies/next",
        "body": {
            "strategy": "sticky_session",
            "session_id": "sticky-sess-99"
        }
    });
    let resp_post = handle_request(req_post, &service);
    assert_eq!(resp_post["status"], 200);
    let post_body: serde_json::Value =
        serde_json::from_str(resp_post["body"].as_str().unwrap()).unwrap();
    assert_eq!(post_body["strategy_applied"], "sticky_session");
}

#[test]
fn test_config_json_sync_and_country_autodetection() {
    let temp_dir = std::env::temp_dir().join(format!("pg_test_cfg_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // 1. Test auto-detection of country from Oxylabs username with special chars in password
    let oxylabs_url = "http://customer-resi_e58ec6d4-cc-vn:_yvFme7=XRnH@pr.oxylabs.io:7777";
    let p_auto = UpstreamProxy::parse(oxylabs_url, None, None, None).unwrap();
    assert_eq!(p_auto.country.as_deref(), Some("VN"));
    assert_eq!(
        p_auto.auth.as_ref().unwrap().username,
        "customer-resi_e58ec6d4-cc-vn"
    );
    assert_eq!(p_auto.auth.as_ref().unwrap().password, "_yvFme7=XRnH");

    // 2. Write a gateway-style config.json in temp_dir
    let config_json = json!({
        "upstream_proxy_pool": "http://customer-resi_e58ec6d4-cc-vn:_yvFme7=XRnH@pr.oxylabs.io:7777\nsocks5://backup:pass123@1.1.1.1:1080",
        "rotation_strategy": "lowest_failures",
        "cost_per_gb_usd": 4.5,
        "block_analytics": true
    });
    std::fs::write(temp_dir.join("config.json"), config_json.to_string()).unwrap();

    // 3. Instantiate FileStorageAdapter on temp_dir
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    );

    // Verify proxies were automatically loaded from config.json
    let proxies = service.list_proxies();
    assert_eq!(proxies.len(), 2);
    assert_eq!(proxies[0].country.as_deref(), Some("VN"));
    assert_eq!(proxies[0].host, "pr.oxylabs.io");
    assert_eq!(proxies[1].host, "1.1.1.1");

    // Verify /proxies/next succeeds with country=VN
    let req = json!({
        "method": "POST",
        "path": "/proxies/next",
        "body": {
            "country": "VN"
        }
    });
    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body: serde_json::Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["proxy"]["country"], "VN");
    assert_eq!(
        body["playwright_config"]["server"],
        "http://pr.oxylabs.io:7777"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_dynamic_country_adaptation_and_shared_endpoint() {
    let temp_dir = std::env::temp_dir().join(format!("pg_pool_adapt_{}", std::process::id()));
    let service = make_service_with_pool(&temp_dir);

    // Add Oxylabs VN residential proxy
    let p_vn = UpstreamProxy::parse(
        "http://customer-resi_e58ec6d4-cc-vn:_yvFme7%3DXRnH@pr.oxylabs.io:7777",
        None,
        None,
        None,
    )
    .unwrap();
    service.add_proxy(p_vn).unwrap();

    // 1. Query for VN -> returns exact match
    let q_vn = ProxySelectionQuery {
        country: Some("VN".to_string()),
        ..Default::default()
    };
    let sel_vn = service.select_proxy(&q_vn).unwrap();
    assert_eq!(sel_vn.country.as_deref(), Some("VN"));
    let pw_vn = sel_vn.to_playwright_config();
    assert_eq!(
        pw_vn.username.as_deref(),
        Some("customer-resi_e58ec6d4-cc-vn")
    );

    // 2. Query for US -> dynamically adapts username from -cc-vn to -cc-us!
    let q_us = ProxySelectionQuery {
        country: Some("US".to_string()),
        ..Default::default()
    };
    let sel_us = service.select_proxy(&q_us).unwrap();
    assert_eq!(sel_us.country.as_deref(), Some("US"));
    let pw_us = sel_us.to_playwright_config();
    assert_eq!(
        pw_us.username.as_deref(),
        Some("customer-resi_e58ec6d4-cc-us")
    );

    // 3. Add explicit US proxy to same host and port -> both coexist with distinct IDs
    let p_us = UpstreamProxy::parse(
        "http://customer-resi_e58ec6d4-cc-us:_yvFme7%3DXRnH@pr.oxylabs.io:7777",
        None,
        None,
        None,
    )
    .unwrap();
    service.add_proxy(p_us).unwrap();

    let all = service.list_proxies();
    assert_eq!(all.len(), 2);
    assert_ne!(all[0].id, all[1].id);

    // 4. Test via HTTP /proxies/next handler
    let req = json!({
        "method": "POST",
        "path": "/proxies/next",
        "body": {
            "country": "US"
        }
    });
    let resp = handle_request(req, &service);
    assert_eq!(resp["status"], 200);
    let body: serde_json::Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["proxy"]["country"], "US");
    assert_eq!(
        body["playwright_config"]["username"],
        "customer-resi_e58ec6d4-cc-us"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_record_proxy_result_tracks_adapted_country_proxy() {
    let temp_dir = std::env::temp_dir().join("proxy_guard_test_adapted_result");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));

    let p_vn = UpstreamProxy::parse(
        "http://customer-resi_test-cc-vn:secret@pr.oxylabs.io:7777",
        None,
        None,
        None,
    )
    .unwrap();
    let base_id = p_vn.id.clone();
    pool.add_proxy(p_vn).unwrap();

    // Select adapted US proxy -> ID becomes `pr.oxylabs.io:7777:customer-resi_test-cc-us`
    let adapted = pool
        .select_proxy(&ProxySelectionQuery {
            country: Some("US".to_string()),
            ..Default::default()
        })
        .unwrap();
    assert_ne!(adapted.id, base_id);

    // Record failure against adapted ID -> base proxy failure_count should increment
    pool.record_proxy_result(&adapted.id, false);
    let list = pool.list_proxies();
    assert_eq!(list[0].failure_count, 1);

    // Record success against adapted ID -> base proxy success_count increments, failure_count decrements
    pool.record_proxy_result(&adapted.id, true);
    let list2 = pool.list_proxies();
    assert_eq!(list2[0].success_count, 1);
    assert_eq!(list2[0].failure_count, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
