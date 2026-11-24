use proxy_guard::domain::ProxyRotationStrategy;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::proxy_server::ProxyServer;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn test_proxy_server_intercepts_http_and_https_connect() {
    let temp_dir = std::env::temp_dir().join(format!("pg_srv_test_{}", std::process::id()));
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
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

    let test_port = 18123;
    let server = ProxyServer::new(test_port, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let _handle = server.start_background().expect("Failed to start proxy");

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

        let mut resp = [0u8; 1024];
        let n = client
            .read(&mut resp)
            .expect("HTTP response read timed out or failed");
        let resp_str = std::str::from_utf8(&resp[..n]).unwrap();

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

        let mut resp = [0u8; 512];
        let n = client
            .read(&mut resp)
            .expect("CONNECT response read timed out or failed");
        let resp_str = std::str::from_utf8(&resp[..n]).unwrap();

        // Must fail closed with 403 Forbidden! Zero bytes forwarded!
        assert!(resp_str.contains("HTTP/1.1 403 Forbidden"));
        assert!(resp_str.contains("Blocked by Proxy Guard"));
    }

    // Signal shutdown
    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_proxy_server_blocks_direct_ssrf() {
    let temp_dir = std::env::temp_dir().join(format!("pg_ssrf_test_{}", std::process::id()));
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
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

    let test_port = 18124;
    let server = ProxyServer::new(test_port, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let _handle = server.start_background().expect("Failed to start proxy");

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

    let mut resp = [0u8; 512];
    let n = client.read(&mut resp).expect("Read failed");
    let resp_str = std::str::from_utf8(&resp[..n]).unwrap();

    // Must return 502 Bad Gateway because direct SSRF connection is blocked by try_connect_target_socket
    assert!(resp_str.contains("HTTP/1.1 502 Bad Gateway"));

    shutdown.store(true, Ordering::Relaxed);
}

#[test]
fn test_proxy_server_origin_form_request() {
    let temp_dir = std::env::temp_dir().join(format!("pg_orig_test_{}", std::process::id()));
    let storage = Arc::new(FileStorageAdapter::new(&temp_dir));
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

    let test_port = 18125;
    let server = ProxyServer::new(test_port, None, Arc::clone(&service));
    let shutdown = server.shutdown_handle();
    let _handle = server.start_background().expect("Failed to start proxy");

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

    let mut resp = [0u8; 1024];
    let n = client.read(&mut resp).expect("Read failed");
    let resp_str = std::str::from_utf8(&resp[..n]).unwrap();

    // Origin-form was reconstructed to http://www.google-analytics.com/analytics.js and mocked!
    assert!(resp_str.contains("HTTP/1.1 200 OK"));
    assert!(resp_str.contains("application/javascript"));

    shutdown.store(true, Ordering::Relaxed);
}
