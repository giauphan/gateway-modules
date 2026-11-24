use proxy_guard::domain::ProxyRotationStrategy;
use proxy_guard::proxy_pool::ProxyPool;
use proxy_guard::proxy_server::ProxyServer;
use proxy_guard::service::ProxyGuardService;
use proxy_guard::storage::FileStorageAdapter;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
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
