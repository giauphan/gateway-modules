use proxy_guard::domain::ProxyRotationStrategy;
use proxy_guard::ports::FlowOptimizerPort;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use std::sync::Arc;

#[test]
fn test_script_generation_targets() {
    let temp_dir = std::env::temp_dir().join(format!("pg_script_test_{}", std::process::id()));
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

    let pw_ts = service.generate_client_script("playwright_ts").unwrap();
    assert!(pw_ts.contains("launchPersistentContext"));
    assert!(pw_ts.contains("enablePageRouteBlocking = false"));
    assert!(pw_ts.contains("disable_non_proxied_udp"));
    assert!(pw_ts.contains("timezoneId"));
    assert!(pw_ts.contains("locale"));
    assert!(pw_ts.contains("userDataDir"));
    assert!(pw_ts.contains("sessionId"));

    let pw_py = service.generate_client_script("playwright_py").unwrap();
    assert!(pw_py.contains("def launch_with_proxy_guard"));
    assert!(pw_py.contains("launch_persistent_context"));
    assert!(pw_py.contains("enable_page_route_blocking: bool = False"));
    assert!(pw_py.contains("disable_non_proxied_udp"));
    assert!(pw_py.contains("timezone_id"));
    assert!(pw_py.contains("user_data_dir"));
    assert!(pw_py.contains("session_id"));

    let puppeteer = service.generate_client_script("puppeteer_ts").unwrap();
    assert!(puppeteer.contains("setRequestInterception(true)"));

    let mitm = service.generate_client_script("mitmproxy_py").unwrap();
    assert!(mitm.contains("class ProxyGuard"));
    assert!(mitm.contains("http.Response.make"));
    assert!(mitm.contains("204"));
    assert!(mitm.contains("--ignore-hosts"));
    assert!(mitm.contains("services\\.sheerid\\.com"));

    assert!(service.generate_client_script("invalid_target").is_err());
}
