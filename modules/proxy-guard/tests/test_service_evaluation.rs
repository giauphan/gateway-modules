use proxy_guard::domain::{
    FlowRequestDto, ProxyRotationStrategy, ProxySelectionQuery, RuleAction, StickySessionConfig,
    UpstreamProxy,
};
use proxy_guard::handlers::handle_request;
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
