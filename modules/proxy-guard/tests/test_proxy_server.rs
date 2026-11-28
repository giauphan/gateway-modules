use proxy_guard::domain::{DefaultRoute, ProxyRotationStrategy, UpstreamProxy};
use proxy_guard::http_cache::HttpCacheConfig;
use proxy_guard::ports::{MetricsPort, ProxyPoolManagementPort, RuleManagementPort};
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::proxy_server::ProxyServer;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}_{}_{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self { path }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn read_full_response(client: &mut TcpStream) -> String {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 512];
    let mut content_length = None;
    let mut header_end = None;

    while buf.len() < 16384 {
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if header_end.is_none() {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let hend = pos + 4;
                        header_end = Some(hend);
                        if let Ok(hdr_str) = std::str::from_utf8(&buf[..hend]) {
                            for line in hdr_str.lines() {
                                if let Some((k, v)) = line.split_once(':') {
                                    if k.trim().eq_ignore_ascii_case("content-length") {
                                        content_length = v.trim().parse::<usize>().ok();
                                    }
                                }
                            }
                        }
                    }
                }

                if let (Some(hend), Some(clen)) = (header_end, content_length) {
                    if buf.len() >= hend + clen {
                        break;
                    }
                }
            }
        }
    }

    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn test_proxy_server_intercepts_http_and_https_connect() {
    let workspace = TestWorkspace::new("pg_srv_test");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    // Bind to port 0 for OS-assigned ephemeral port
    let server = ProxyServer::new(0, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("Failed to start proxy");

    thread::sleep(Duration::from_millis(50));

    // 1. Test Plain HTTP interception of Google Analytics
    {
        let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}"))
            .expect("Failed to connect to proxy for HTTP test");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("Failed to set read timeout on HTTP client stream");
        client
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("Failed to set write timeout on HTTP client stream");

        let req = "GET http://www.google-analytics.com/analytics.js HTTP/1.1\r\nHost: www.google-analytics.com\r\n\r\n";
        client
            .write_all(req.as_bytes())
            .expect("HTTP request write timed out or failed");

        let resp_str = read_full_response(&mut client);

        assert!(resp_str.contains("HTTP/1.1 200 OK"));
        assert!(resp_str.contains("application/javascript"));
        assert!(resp_str.contains("dataLayer"));
    }

    // 2. Test HTTPS CONNECT interception of Facebook Pixel
    {
        let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}"))
            .expect("Failed to connect to proxy for CONNECT test");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("Failed to set read timeout on CONNECT client stream");
        client
            .set_write_timeout(Some(Duration::from_secs(5)))
            .expect("Failed to set write timeout on CONNECT client stream");

        let req =
            "CONNECT connect.facebook.net:443 HTTP/1.1\r\nHost: connect.facebook.net:443\r\n\r\n";
        client
            .write_all(req.as_bytes())
            .expect("CONNECT request write timed out or failed");

        let resp_str = read_full_response(&mut client);

        // Must fail closed with 403 Forbidden! Zero bytes forwarded!
        assert!(resp_str.contains("HTTP/1.1 403 Forbidden"));
        assert!(resp_str.contains("Blocked by Proxy Guard"));
    }

    // Signal shutdown
    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_proxy_server_blocks_direct_ssrf() {
    let workspace = TestWorkspace::new("pg_ssrf_test");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    // Bind to port 0 for OS-assigned ephemeral port
    let server = ProxyServer::new(0, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("Failed to start proxy");

    thread::sleep(Duration::from_millis(50));

    // Client requests cloud instance metadata or loopback directly without an upstream proxy
    let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}"))
        .expect("Failed to connect to proxy for SSRF test");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("Failed to set read timeout");
    client
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("Failed to set write timeout");

    let req =
        "GET http://169.254.169.254/latest/meta-data/ HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n";
    client.write_all(req.as_bytes()).expect("Write failed");

    let resp_str = read_full_response(&mut client);

    // Must return 502 Bad Gateway because direct SSRF connection is blocked by try_connect_target_socket
    assert!(resp_str.contains("HTTP/1.1 502 Bad Gateway"));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_proxy_server_origin_form_request() {
    let workspace = TestWorkspace::new("pg_orig_test");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    // Bind to port 0 for OS-assigned ephemeral port
    let server = ProxyServer::new(0, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("Failed to start proxy");

    thread::sleep(Duration::from_millis(50));

    let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}"))
        .expect("Failed to connect to proxy for origin form test");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("Failed to set read timeout");
    client
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("Failed to set write timeout");

    // Origin-form request target: /analytics.js with Host header
    let req = "GET /analytics.js HTTP/1.1\r\nHost: www.google-analytics.com\r\n\r\n";
    client.write_all(req.as_bytes()).expect("Write failed");

    let resp_str = read_full_response(&mut client);

    // Origin-form was reconstructed to http://www.google-analytics.com/analytics.js and mocked!
    assert!(resp_str.contains("HTTP/1.1 200 OK"));
    assert!(resp_str.contains("application/javascript"));
    assert!(
        resp_str.contains("dataLayer"),
        "Mock body must contain dataLayer stub"
    );

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_rejected_connect_makes_zero_upstream_connections() {
    let workspace = TestWorkspace::new("pg_connect_reject_zero");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    // Mock upstream proxy listener that counts every TCP accept
    let upstream_listener = TcpListener::bind("127.0.0.1:0").expect("bind mock upstream");
    let upstream_port = upstream_listener.local_addr().expect("addr").port();
    let _ = upstream_listener.set_nonblocking(true);
    let upstream_accepts = Arc::new(AtomicUsize::new(0));
    let upstream_stop = Arc::new(AtomicBool::new(false));

    let accepts_clone = Arc::clone(&upstream_accepts);
    let stop_clone = Arc::clone(&upstream_stop);
    let upstream_thread = thread::spawn(move || {
        while !stop_clone.load(Ordering::Relaxed) {
            if upstream_listener.accept().is_ok() {
                accepts_clone.fetch_add(1, Ordering::Relaxed);
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    let upstream_url = format!("http://127.0.0.1:{upstream_port}");
    service
        .add_proxy(
            UpstreamProxy::parse(&upstream_url, Some("mock-up"), Some("US"), None).expect("parse"),
        )
        .expect("add proxy");

    let server = ProxyServer::new(0, Some(upstream_url), Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("start proxy");
    thread::sleep(Duration::from_millis(40));

    let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}")).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    client
        .write_all(b"CONNECT www.google-analytics.com:443 HTTP/1.1\r\nHost: www.google-analytics.com:443\r\n\r\n")
        .expect("write");

    let resp = read_full_response(&mut client);
    assert!(resp.contains("HTTP/1.1 403 Forbidden"));
    assert_eq!(
        upstream_accepts.load(Ordering::Relaxed),
        0,
        "Rejected CONNECT must open 0 upstream connections"
    );
    let metrics = service.get_metrics();
    assert_eq!(metrics.blocked_requests, 1);
    assert_eq!(metrics.bytes_tx, 0);
    assert_eq!(metrics.bytes_rx, 0);

    shutdown.store(true, Ordering::Relaxed);
    upstream_stop.store(true, Ordering::Relaxed);
    let _ = upstream_thread.join();
}

#[test]
fn test_direct_split_route_enforces_ssrf_on_loopback_and_private_ips() {
    let workspace = TestWorkspace::new("pg_direct_ssrf");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));
    service
        .set_default_route(DefaultRoute::Direct)
        .expect("set direct");

    let server = ProxyServer::new(0, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("start proxy");
    thread::sleep(Duration::from_millis(40));

    for target in ["127.0.0.1:80", "10.0.0.1:443", "169.254.169.254:80"] {
        let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}")).expect("connect");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let req = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
        client.write_all(req.as_bytes()).expect("write");
        let resp = read_full_response(&mut client);
        assert!(
            resp.contains("HTTP/1.1 502 Bad Gateway"),
            "SSRF target {target} must be blocked on Direct route, got: {resp}"
        );
    }

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_http_response_cache_hits_and_mandatory_bypasses() {
    let workspace = TestWorkspace::new("pg_http_cache_test");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    service.configure_cache(HttpCacheConfig {
        enabled: true,
        max_entries: 64,
        max_body_bytes: 65536,
        default_ttl_secs: 120,
        ..Default::default()
    });

    // Mock upstream HTTP proxy that serves a 200 OK JSON body and counts every hit
    let upstream_listener = TcpListener::bind("127.0.0.1:0").expect("bind mock upstream");
    let upstream_port = upstream_listener.local_addr().expect("addr").port();
    let _ = upstream_listener.set_nonblocking(true);
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let upstream_stop = Arc::new(AtomicBool::new(false));

    let hits_clone = Arc::clone(&upstream_hits);
    let stop_clone = Arc::clone(&upstream_stop);
    let upstream_thread = thread::spawn(move || {
        while !stop_clone.load(Ordering::Relaxed) {
            if let Ok((mut stream, _)) = upstream_listener.accept() {
                let count = hits_clone.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut req_buf = [0u8; 2048];
                let _ = stream.read(&mut req_buf);
                let body = format!("{{\"schools\":[\"MIT\"],\"hit\":{count}}}");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: public, max-age=120\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    let upstream_url = format!("http://127.0.0.1:{upstream_port}");
    service
        .add_proxy(
            UpstreamProxy::parse(&upstream_url, Some("mock-up"), Some("US"), None).expect("parse"),
        )
        .expect("add proxy");

    let server = ProxyServer::new(0, Some(upstream_url), Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("start proxy");
    thread::sleep(Duration::from_millis(40));

    let send_req = |raw: &str| -> String {
        let mut c = TcpStream::connect(format!("127.0.0.1:{test_port}")).expect("connect");
        c.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        c.write_all(raw.as_bytes()).expect("write");
        read_full_response(&mut c)
    };

    // 1. First GET -> miss -> upstream hit #1
    let r1 = send_req("GET http://orgsearch.example.com/schools?q=mit HTTP/1.1\r\nHost: orgsearch.example.com\r\n\r\n");
    assert!(r1.contains("\"hit\":1"));
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

    // 2. Second identical GET -> cache HIT -> upstream STILL 1!
    let r2 = send_req("GET http://orgsearch.example.com/schools?q=mit HTTP/1.1\r\nHost: orgsearch.example.com\r\n\r\n");
    assert!(r2.contains("\"hit\":1"));
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 1);

    // 3. GET with Cookie -> MUST bypass cache -> upstream hit #2
    let r3 = send_req("GET http://orgsearch.example.com/schools?q=mit HTTP/1.1\r\nHost: orgsearch.example.com\r\nCookie: sid=secret\r\n\r\n");
    assert!(r3.contains("\"hit\":2"));
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 2);

    // 4. GET with Authorization -> MUST bypass cache -> upstream hit #3
    let r4 = send_req("GET http://orgsearch.example.com/schools?q=mit HTTP/1.1\r\nHost: orgsearch.example.com\r\nAuthorization: Bearer tok123\r\n\r\n");
    assert!(r4.contains("\"hit\":3"));
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 3);

    // 5. POST -> MUST bypass cache -> upstream hit #4
    let r5 = send_req("POST http://orgsearch.example.com/schools?q=mit HTTP/1.1\r\nHost: orgsearch.example.com\r\nContent-Length: 2\r\n\r\n{}");
    assert!(r5.contains("\"hit\":4"));
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 4);

    // 6. Sensitive path (/login) -> MUST bypass cache -> upstream hit #5 & #6
    let _ = send_req("GET http://orgsearch.example.com/api/login HTTP/1.1\r\nHost: orgsearch.example.com\r\n\r\n");
    let _ = send_req("GET http://orgsearch.example.com/api/login HTTP/1.1\r\nHost: orgsearch.example.com\r\n\r\n");
    assert_eq!(upstream_hits.load(Ordering::Relaxed), 6);

    let metrics = service.get_metrics();
    assert_eq!(metrics.cache_hits, 1);
    assert_eq!(metrics.cache_misses, 1);
    assert!(metrics.cache_bytes_saved > 0);

    shutdown.store(true, Ordering::Relaxed);
    upstream_stop.store(true, Ordering::Relaxed);
    let _ = upstream_thread.join();
}

#[test]
fn test_connect_automatically_fails_over_on_upstream_407() {
    let workspace = TestWorkspace::new("pg_connect_407_failover");
    let storage = Arc::new(FileStorageAdapter::new(&workspace.path));
    let pool = Arc::new(ProxyPool::new(
        Arc::clone(&storage) as Arc<_>,
        ProxyRotationStrategy::RoundRobin,
    ));
    let service = Arc::new(ProxyGuardService::new(
        Arc::clone(&storage) as Arc<_>,
        Arc::clone(&storage) as Arc<_>,
        pool,
        3.5,
    ));

    // Upstream #1: always returns HTTP 407 Proxy Authentication Required
    let bad_listener = TcpListener::bind("127.0.0.1:0").expect("bind 407 mock");
    let bad_port = bad_listener.local_addr().expect("bad addr").port();
    let _ = bad_listener.set_nonblocking(true);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_bad = Arc::clone(&stop_flag);
    let bad_thread = thread::spawn(move || {
        while !stop_bad.load(Ordering::Relaxed) {
            if let Ok((mut s, _)) = bad_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                );
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    // Upstream #2: returns HTTP 200 Connection Established and echoes payload
    let good_listener = TcpListener::bind("127.0.0.1:0").expect("bind 200 mock");
    let good_port = good_listener.local_addr().expect("good addr").port();
    let _ = good_listener.set_nonblocking(true);
    let stop_good = Arc::clone(&stop_flag);
    let good_thread = thread::spawn(move || {
        while !stop_good.load(Ordering::Relaxed) {
            if let Ok((mut s, _)) = good_listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
                let _ = s.flush();
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    service
        .add_proxy(
            UpstreamProxy::parse(
                &format!("http://customer-resi_dead:p1@127.0.0.1:{bad_port}"),
                Some("dead-407"),
                Some("JP"),
                None,
            )
            .expect("parse bad"),
        )
        .expect("add bad");
    service
        .add_proxy(
            UpstreamProxy::parse(
                &format!("http://user-dc_good:p2@127.0.0.1:{good_port}"),
                Some("good-200"),
                Some("JP"),
                None,
            )
            .expect("parse good"),
        )
        .expect("add good");

    let server = ProxyServer::new(0, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let (_handle, test_port) = server.start_background().expect("start proxy");
    thread::sleep(Duration::from_millis(40));

    let mut client = TcpStream::connect(format!("127.0.0.1:{test_port}")).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    client
        .write_all(b"CONNECT services.sheerid.com:443 HTTP/1.1\r\nHost: services.sheerid.com:443\r\nX-Proxy-Country: JP\r\n\r\n")
        .expect("write");

    let resp = read_full_response(&mut client);
    assert!(
        resp.contains("HTTP/1.1 200 Connection Established"),
        "CONNECT must automatically fail over from 407 proxy to healthy backup proxy, got: {resp}"
    );

    // Dead 407 proxy must be immediately quarantined (failure_count >= 3)
    let dead = service
        .list_proxies()
        .into_iter()
        .find(|p| p.port == bad_port)
        .expect("dead proxy entry");
    assert!(
        dead.failure_count >= 3,
        "HTTP 407 proxy must be quarantined immediately, got failure_count={}",
        dead.failure_count
    );

    shutdown.store(true, Ordering::Relaxed);
    stop_flag.store(true, Ordering::Relaxed);
    let _ = bad_thread.join();
    let _ = good_thread.join();
}
