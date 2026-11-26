//! Tests for SSRF protection and proxy server utility functions.

use proxy_guard::proxy_server::utils::{
    extract_host_from_headers, extract_host_port, extract_proxy_query_from_headers,
    find_header_end, is_chunked_transfer_encoding, is_private_or_restricted, parse_content_length,
    split_host_port,
};
use proxy_guard::rules_engine::extract_host;
use std::net::IpAddr;

#[test]
fn test_ssrf_protection_blocked_ips() {
    let restricted_ips = [
        // IPv4 Loopback
        "127.0.0.1",
        "127.0.0.2",
        "127.255.255.255",
        // RFC 1918 Private ranges
        "10.0.0.1",
        "10.254.0.1",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.0.1",
        "192.168.1.100",
        // Carrier-Grade NAT (RFC 6598: 100.64.0.0/10)
        "100.64.0.1",
        "100.100.50.1",
        "100.127.255.254",
        // Link-Local (Cloud metadata service)
        "169.254.169.254",
        "169.254.1.1",
        // Broadcast / Current network
        "0.0.0.0",
        "255.255.255.255",
        // Multicast
        "224.0.0.1",
        "239.255.255.250",
        // IPv6 Loopback & Unspecified
        "::1",
        "::",
        // IPv6 Unique Local (fc00::/7)
        "fc00::1",
        "fd12:3456:789a::1",
        // IPv6 Link-Local (fe80::/10)
        "fe80::1",
        // IPv6 Multicast
        "ff02::1",
        // Dual-Stack IPv4-mapped IPv6 (must NOT bypass IPv6 checks!)
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "::ffff:10.0.0.1",
        "::ffff:192.168.1.1",
        "::ffff:100.64.0.1",
    ];

    for ip_str in restricted_ips {
        let ip: IpAddr = ip_str
            .parse()
            .unwrap_or_else(|_| panic!("Failed to parse {ip_str}"));
        assert!(
            is_private_or_restricted(&ip),
            "SSRF check must BLOCK restricted IP {ip_str}"
        );
    }
}

#[test]
fn test_ssrf_protection_allowed_public_ips() {
    let public_ips = [
        "8.8.8.8",
        "8.8.4.4",
        "1.1.1.1",
        "1.0.0.1",
        "93.184.216.34", // example.com
        "142.250.190.46",
        "2606:4700:4700::1111", // Cloudflare DNS IPv6
        "2001:4860:4860::8888", // Google DNS IPv6
    ];

    for ip_str in public_ips {
        let ip: IpAddr = ip_str
            .parse()
            .unwrap_or_else(|_| panic!("Failed to parse {ip_str}"));
        assert!(
            !is_private_or_restricted(&ip),
            "SSRF check must ALLOW public IP {ip_str}"
        );
    }
}

#[test]
fn test_host_extraction_and_port_formatting() {
    assert_eq!(extract_host("https://example.com/foo/bar"), "example.com");
    assert_eq!(
        extract_host("http://sub.domain.org:8080/query?a=b"),
        "sub.domain.org"
    );
    assert_eq!(extract_host("example.com:443"), "example.com");

    assert_eq!(extract_host_port("example.com", 80), "example.com:80");
    assert_eq!(
        extract_host_port("http://example.com:8080/path", 80),
        "example.com:8080"
    );
    assert_eq!(extract_host_port("1.2.3.4:9000", 80), "1.2.3.4:9000");
}

#[test]
fn test_header_end_detection() {
    let headers_crlf = b"GET / HTTP/1.1\r\nHost: a.com\r\n\r\n";
    assert_eq!(find_header_end(headers_crlf), Some(headers_crlf.len()));
    let headers_lf = b"GET / HTTP/1.1\nHost: a.com\n\n";
    assert_eq!(find_header_end(headers_lf), Some(headers_lf.len()));
    assert_eq!(find_header_end(b"Incomplete header buffer"), None);
}

#[test]
fn test_proxy_query_header_extraction() {
    let headers = "GET / HTTP/1.1\r\n\
        Host: example.com\r\n\
        X-Proxy-Country: US\r\n\
        X-Proxy-Tag: residential\r\n\
        X-Proxy-Session: session-worker-42\r\n\
        X-Proxy-Id: proxy-us-east-1\r\n\r\n";

    let query = extract_proxy_query_from_headers(headers);
    assert_eq!(query.country.as_deref(), Some("US"));
    assert_eq!(query.tag.as_deref(), Some("residential"));
    assert_eq!(query.session_id.as_deref(), Some("session-worker-42"));
    assert_eq!(query.preferred_id.as_deref(), Some("proxy-us-east-1"));

    let empty_headers = "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let empty_query = extract_proxy_query_from_headers(empty_headers);
    assert!(empty_query.country.is_none());
    assert!(empty_query.session_id.is_none());
}

#[test]
fn test_extract_host_from_headers() {
    let headers = b"GET /path HTTP/1.1\r\nUser-Agent: bot\r\nHost: shop.example.com\r\n\r\n";
    assert_eq!(
        extract_host_from_headers(headers).as_deref(),
        Some("shop.example.com")
    );

    let headers_with_port = b"GET / HTTP/1.1\r\nHost: api.example.com:8443\r\n\r\n";
    assert_eq!(
        extract_host_from_headers(headers_with_port).as_deref(),
        Some("api.example.com:8443")
    );

    let no_host = b"GET / HTTP/1.1\r\nUser-Agent: bot\r\n\r\n";
    assert!(extract_host_from_headers(no_host).is_none());
}

#[test]
fn test_http_transfer_encoding_and_content_length() {
    let headers = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 42\r\n\r\n";
    assert!(is_chunked_transfer_encoding(headers));
    assert_eq!(parse_content_length(headers), Some(42));

    let standard = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n";
    assert!(!is_chunked_transfer_encoding(standard));
    assert_eq!(parse_content_length(standard), None);
}

#[test]
fn test_ipv6_and_userinfo_host_port_parsing() {
    assert_eq!(extract_host_port("http://[::1]/foo", 80), "[::1]:80");
    assert_eq!(
        extract_host_port("https://[::1]:8443/path", 443),
        "[::1]:8443"
    );
    assert_eq!(
        extract_host_port("http://user:pass@example.com/path", 80),
        "example.com:80"
    );

    assert_eq!(split_host_port("[::1]:443", 80), ("::1", 443));
    assert_eq!(split_host_port("[::1]", 8080), ("::1", 8080));
    assert_eq!(split_host_port("example.com:8443", 80), ("example.com", 8443));
    assert_eq!(split_host_port("example.com", 80), ("example.com", 80));
}
