use proxy_guard::domain::{
    DefaultRoute, FlowRequestDto, ProxyRotationStrategy, ProxySelectionQuery, RuleAction,
    StickySessionConfig, UpstreamProxy,
};
use proxy_guard::handlers::handle_request;
use proxy_guard::mcp::handle_mcp_tool_request;
use proxy_guard::ports::{
    FlowOptimizerPort, MetricsPort, ProxyPoolManagementPort, RuleManagementPort,
};
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use std::sync::Arc;

#[test]
fn test_flow_optimization_drastic_reduction() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
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

    // Simulate a typical web flow that consumes ~35MB:
    // - 1 HTML document (50KB) - allowed
    // - 2 Application API JSON calls (10KB each) - allowed
    // - 1 Heavy MP4 background video (25MB) - blocked!
    // - 4 Analytics / tracker calls (GTM, GA, Hotjar, Sentry) - blocked!
    // - 2 Custom Web Fonts (1.5MB total) - blocked!
    let flow_requests = vec![
        FlowRequestDto {
            url: "https://shop.example.com/checkout".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("document".to_string()),
            size_bytes: Some(50_000),
        },
        FlowRequestDto {
            url: "https://api.shop.example.com/v1/cart".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("xhr".to_string()),
            size_bytes: Some(12_000),
        },
        FlowRequestDto {
            url: "https://api.shop.example.com/v1/auth/session".to_string(),
            method: Some("POST".to_string()),
            resource_type: Some("xhr".to_string()),
            size_bytes: Some(8_000),
        },
        FlowRequestDto {
            url: "https://cdn.shop.example.com/videos/hero_background.mp4".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("media".to_string()),
            size_bytes: Some(26_000_000), // 26MB video!
        },
        FlowRequestDto {
            url: "https://www.googletagmanager.com/gtm.js?id=GTM-TEST".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("script".to_string()),
            size_bytes: Some(150_000),
        },
        FlowRequestDto {
            url: "https://www.google-analytics.com/g/collect".to_string(),
            method: Some("POST".to_string()),
            resource_type: Some("beacon".to_string()),
            size_bytes: Some(1_200),
        },
        FlowRequestDto {
            url: "https://static.hotjar.com/c/hotjar-999.js".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("script".to_string()),
            size_bytes: Some(80_000),
        },
        FlowRequestDto {
            url: "https://ingest.sentry.io/api/123/envelope/".to_string(),
            method: Some("POST".to_string()),
            resource_type: Some("xhr".to_string()),
            size_bytes: Some(3_500),
        },
        FlowRequestDto {
            url: "https://cdn.shop.example.com/fonts/inter-bold.woff2".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("font".to_string()),
            size_bytes: Some(800_000),
        },
        FlowRequestDto {
            url: "https://cdn.shop.example.com/fonts/inter-regular.woff2".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("font".to_string()),
            size_bytes: Some(750_000),
        },
    ];

    let result = service.optimize_flow(&flow_requests);

    assert_eq!(result.total_flow_requests, 10);
    assert_eq!(result.forwarded_requests, 3); // only document & 2 core APIs forwarded
    assert_eq!(result.blocked_requests, 7); // video, analytics, telemetry, fonts blocked

    // Original flow was ~27.8MB; optimized flow is only 70KB!
    assert!(
        result.reduction_percentage > 95.0,
        "Expected >95% reduction, got {}%",
        result.reduction_percentage
    );
    assert!(result.cost_saved_usd > 0.0);

    let metrics = service.get_metrics();
    assert_eq!(metrics.blocked_requests, 7);
    assert_eq!(metrics.forwarded_requests, 3);
}

#[test]
fn test_dynamic_split_routing_direct_bypass_and_savings() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_split_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
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

    // Register a custom CDN for direct split-routing (0 GB paid proxy)
    service
        .add_direct_bypass_domain("cdn.jsdelivr.net", Some("jsDelivr Public CDN"))
        .expect("should add direct bypass rule");

    let direct_decision = service.evaluate_url(
        "https://cdn.jsdelivr.net/npm/app.min.js",
        "GET",
        Some("script"),
    );
    assert_eq!(direct_decision.action, RuleAction::BypassDirect);
    assert!(!direct_decision.is_blocked());

    // Sensitive anti-bot platforms MUST stay ForwardUpstream (through paid proxy)
    let sheerid = service.evaluate_url(
        "https://services.sheerid.com/rest/v2/verification",
        "POST",
        Some("xhr"),
    );
    assert_eq!(sheerid.action, RuleAction::ForwardUpstream);

    let muse = service.evaluate_url("https://api.muse.ai/v1/session", "GET", Some("xhr"));
    assert_eq!(muse.action, RuleAction::ForwardUpstream);

    // Verify optimize_flow counts BypassDirect as 0 paid-proxy bytes
    let flow = vec![
        FlowRequestDto {
            url: "https://services.sheerid.com/rest/v2/verification".to_string(),
            method: Some("POST".to_string()),
            resource_type: Some("xhr".to_string()),
            size_bytes: Some(20_000),
        },
        FlowRequestDto {
            url: "https://cdn.jsdelivr.net/npm/bundle.js".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("script".to_string()),
            size_bytes: Some(2_000_000),
        },
        FlowRequestDto {
            url: "https://static.xx.fbcdn.net/rsrc.php/v3/core.js".to_string(),
            method: Some("GET".to_string()),
            resource_type: Some("script".to_string()),
            size_bytes: Some(3_000_000),
        },
    ];

    let result = service.optimize_flow(&flow);
    assert_eq!(result.total_flow_requests, 3);
    assert_eq!(result.forwarded_requests, 1);
    assert!(
        result.reduction_percentage > 99.0,
        "Expected >99% paid proxy reduction with BypassDirect, got {}%",
        result.reduction_percentage
    );

    let metrics = service.get_metrics();
    assert!(metrics.direct_bypassed_requests >= 3);
    assert!(metrics.bytes_saved >= 5_000_000);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sticky_session_template_ttl_rotation_and_credential_masking() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_sticky_ttl_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::StickySession,
    ));
    let service = ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    );

    let resi = UpstreamProxy::parse(
        "http://di_user:SUPER_SECRET_PASS_99@gw.dataimpulse.com:823",
        Some("dataimpulse-us"),
        Some("US"),
        Some(vec!["residential".to_string()]),
    )
    .expect("parse")
    .with_pool_and_sticky(
        Some("residential".to_string()),
        Some(StickySessionConfig {
            enabled: true,
            ttl_secs: 1, // 1s TTL for fast expiry rotation test
            username_template: Some(
                "{base_user}_cr.{country}_sid.{session_id}_ttl.{ttl_mins}m".to_string(),
            ),
        }),
    );
    service.add_proxy(resi).expect("add proxy");

    // 1. Within TTL: same session_id produces identical sticky username
    let q_a = ProxySelectionQuery {
        pool: Some("residential".to_string()),
        session_id: Some("sheerid-user-1".to_string()),
        country: Some("US".to_string()),
        ..Default::default()
    };
    let p1 = service.select_proxy(&q_a).expect("select p1");
    let p2 = service.select_proxy(&q_a).expect("select p2");
    let u1 = p1.auth.as_ref().expect("auth1").username.clone();
    let u2 = p2.auth.as_ref().expect("auth2").username.clone();
    assert_eq!(u1, u2, "Sticky username must stay stable within TTL");
    assert!(u1.starts_with("di_user_cr.us_sid."));
    assert!(u1.ends_with("_ttl.1m"));

    // 2. Different session_id rotates immediately
    let q_b = ProxySelectionQuery {
        pool: Some("residential".to_string()),
        session_id: Some("sheerid-user-2".to_string()),
        country: Some("US".to_string()),
        ..Default::default()
    };
    let p_b = service.select_proxy(&q_b).expect("select p_b");
    let u_b = p_b.auth.as_ref().expect("auth_b").username.clone();
    assert_ne!(u1, u_b, "Changing session_id must rotate sticky token");

    // 3. After TTL expires: same session_id rotates cleanly
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let p3 = service.select_proxy(&q_a).expect("select p3 after expiry");
    let u3 = p3.auth.as_ref().expect("auth3").username.clone();
    assert_ne!(
        u1, u3,
        "Sticky session must rotate cleanly after TTL expiry"
    );

    // 4. Auto-generated internal session when client omits session_id
    let q_auto = ProxySelectionQuery {
        pool: Some("residential".to_string()),
        ..Default::default()
    };
    let p_auto_1 = service.select_proxy(&q_auto).expect("auto 1");
    let p_auto_2 = service.select_proxy(&q_auto).expect("auto 2");
    assert_eq!(
        p_auto_1.auth.as_ref().expect("a1").username,
        p_auto_2.auth.as_ref().expect("a2").username
    );

    // 5. Security: list_proxies API response MUST NOT leak plaintext password
    let list_resp = handle_request(
        serde_json::json!({ "method": "GET", "path": "/proxies/list" }),
        &service,
    );
    let body_str = list_resp["body"].as_str().unwrap_or("");
    assert!(
        !body_str.contains("SUPER_SECRET_PASS_99"),
        "Plaintext proxy password must NEVER leak in API responses"
    );
    assert!(body_str.contains("***"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v1_15_2_pool_route_update_overwrites_without_duplicates() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_pool_overwrite_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
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

    service
        .add_pool_route_domain("services.sheerid.com", "residential", None)
        .expect("add initial pool route");
    assert_eq!(
        service
            .evaluate_url("https://services.sheerid.com/rest/v2", "GET", None)
            .action,
        RuleAction::RoutePool("residential".to_string())
    );

    // Re-route the same domain to a different pool -> MUST overwrite cleanly
    service
        .add_pool_route_domain("services.sheerid.com", "static_isp_us", None)
        .expect("update pool route");
    assert_eq!(
        service
            .evaluate_url("https://services.sheerid.com/rest/v2", "GET", None)
            .action,
        RuleAction::RoutePool("static_isp_us".to_string())
    );

    let matching_rules: Vec<_> = service
        .get_rules()
        .into_iter()
        .filter(|r| {
            r.id == "pool:exact:services.sheerid.com" || r.pattern == "services.sheerid.com"
        })
        .collect();
    assert_eq!(
        matching_rules.len(),
        1,
        "Updating a domain's pool must not leave duplicate rules"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v1_15_2_malformed_sticky_session_rejected_and_invalid_default_route() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_validation_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);
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

    // 1. Malformed sticky_session (ttl_secs is string instead of u64) via HTTP POST /proxies/add
    let http_bad = handle_request(
        serde_json::json!({
            "method": "POST",
            "path": "/proxies/add",
            "body": {
                "url": "http://u:p@gw.example.com:823",
                "sticky_session": { "enabled": true, "ttl_secs": "600" }
            }
        }),
        &service,
    );
    assert_eq!(http_bad["status"], 400);
    assert!(http_bad["body"]
        .as_str()
        .unwrap_or("")
        .contains("sticky_session"));

    // 2. Malformed sticky_session via MCP tool_add_upstream_proxy
    let mcp_bad = handle_mcp_tool_request(
        "proxy-guard__add_upstream_proxy",
        &serde_json::json!({
            "url": "http://u:p@gw.example.com:823",
            "sticky_session": { "enabled": "yes", "ttl_secs": 600 }
        }),
        &service,
    );
    assert_eq!(mcp_bad["isError"], true);

    // 3. Invalid default_route keywords rejected
    for bad in [
        "reject",
        "block",
        "deny",
        "drop",
        "none",
        "allow",
        "",
        "bad pool!",
    ] {
        assert!(
            DefaultRoute::parse_str(bad).is_err(),
            "Expected '{bad}' to be rejected as invalid default_route"
        );
    }
    assert_eq!(
        DefaultRoute::parse_str("pool:residential").expect("valid pool prefix"),
        DefaultRoute::Pool("residential".to_string())
    );
    assert_eq!(
        DefaultRoute::parse_str("static_isp_us").expect("valid bare pool"),
        DefaultRoute::Pool("static_isp_us".to_string())
    );

    let rules_bad_dr = handle_request(
        serde_json::json!({
            "method": "POST",
            "path": "/rules",
            "body": { "default_route": "reject" }
        }),
        &service,
    );
    assert_eq!(rules_bad_dr["status"], 400);

    let mcp_bad_dr = handle_mcp_tool_request(
        "proxy-guard__configure_rules",
        &serde_json::json!({ "default_route": "block" }),
        &service,
    );
    assert_eq!(mcp_bad_dr["isError"], true);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v1_15_2_default_route_and_transfer_cache_metrics_persist_across_restart() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_persist_restart_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

    // First service lifecycle: set default_route and record >64 transfer/cache events to trigger batch persistence
    {
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

        service
            .set_default_route(DefaultRoute::Direct)
            .expect("persist default_route");
        for _ in 0..70 {
            service.record_upstream_transfer("residential", 100, 500);
            service.record_cache_hit(1024);
            service.record_cache_miss();
        }
    }

    // Second service lifecycle (simulated restart): reload from disk
    let storage2 = Arc::new(FileStorageAdapter::new(&temp_dir));
    let pool2 = Arc::new(ProxyPool::new(
        Arc::clone(&storage2) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service2 = ProxyGuardService::new(
        Arc::clone(&storage2) as Arc<_>,
        Arc::clone(&storage2) as Arc<_>,
        pool2,
        3.5,
    );

    assert_eq!(service2.get_default_route(), DefaultRoute::Direct);
    assert_eq!(
        service2
            .evaluate_url("https://unmatched-static.example.org/file.js", "GET", None)
            .action,
        RuleAction::BypassDirect
    );

    let loaded_metrics = service2.get_metrics();
    assert_eq!(loaded_metrics.bytes_tx, 7_000);
    assert_eq!(loaded_metrics.bytes_rx, 35_000);
    assert_eq!(loaded_metrics.cache_hits, 70);
    assert_eq!(loaded_metrics.cache_misses, 70);
    assert_eq!(loaded_metrics.cache_bytes_saved, 70 * 1024);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_v1_15_3_short_run_metrics_flush_and_example_config_ip_alignment() {
    let temp_dir = std::env::temp_dir().join(format!(
        "pg_short_flush_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::create_dir_all(&temp_dir);

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

    // 1. Short run (16 updates, well below the old 64 threshold) flushes to disk without waiting for Drop
    for _ in 0..16 {
        service.record_upstream_transfer("residential", 50, 200);
    }
    let storage_reader = FileStorageAdapter::new(&temp_dir);
    let on_disk = proxy_guard::ports::MetricsStoragePort::load_metrics(&storage_reader);
    assert_eq!(on_disk.bytes_tx, 800);
    assert_eq!(on_disk.bytes_rx, 3200);

    // 2. Verify config.example.json IP-alignment rules for SheerID + CAPTCHA and Muse.ai + Meta
    let example_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config.example.json");
    let raw = std::fs::read_to_string(&example_path).expect("read config.example.json");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse config.example.json");

    if let Some(dr_val) = cfg.get("default_route") {
        let route = DefaultRoute::from_json_value(dr_val).expect("valid default_route");
        service.set_default_route(route).expect("set default_route");
    }
    for item in cfg["routing_rules"]
        .as_array()
        .expect("routing_rules array")
    {
        let domain = item["domain"].as_str().expect("domain");
        let action = item["action"].as_str().expect("action");
        match action {
            "pool" => service
                .add_pool_route_domain(domain, item["pool"].as_str().expect("pool"), None)
                .expect("add pool route"),
            "direct" => service
                .add_direct_bypass_domain(domain, None)
                .expect("add direct route"),
            "reject" => service
                .add_blocked_domain(domain, None)
                .expect("add reject route"),
            _ => {}
        }
    }

    // SheerID + CAPTCHAs all share residential pool
    for host in [
        "https://services.sheerid.com/rest/v2/verification",
        "https://my.sheerid.com/verify",
        "https://challenges.cloudflare.com/turnstile/v0/api.js",
        "https://api.hcaptcha.com/checksiteconfig",
        "https://www.recaptcha.net/recaptcha/api.js",
    ] {
        assert_eq!(
            service.evaluate_url(host, "GET", None).action,
            RuleAction::RoutePool("residential".to_string()),
            "{host} must route to residential pool"
        );
    }

    // Muse.ai + Meta auth + Stripe telemetry all share static_isp_us pool
    for host in [
        "https://muse.ai/app",
        "https://auth.muse.ai/login",
        "https://auth.meta.com/oauth",
        "https://accountscenter.meta.com/profiles",
        "https://m.stripe.com/6",
    ] {
        assert_eq!(
            service.evaluate_url(host, "GET", None).action,
            RuleAction::RoutePool("static_isp_us".to_string()),
            "{host} must route to static_isp_us pool"
        );
    }

    // Public CDNs stay DIRECT, trackers REJECT
    assert_eq!(
        service
            .evaluate_url(
                "https://static.xx.fbcdn.net/rsrc.php/v3/core.js",
                "GET",
                None
            )
            .action,
        RuleAction::BypassDirect
    );
    assert!(service
        .evaluate_url("https://www.google-analytics.com/g/collect", "POST", None)
        .is_blocked());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
