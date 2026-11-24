use proxy_guard::domain::{FlowRequestDto, ProxyRotationStrategy};
use proxy_guard::ports::{FlowOptimizerPort, MetricsPort};
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
