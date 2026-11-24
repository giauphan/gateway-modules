//! Native Intercepting Proxy Server.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Runs a local HTTP/CONNECT proxy that intercepts traffic before it reaches paid upstream proxies.
//! Implements multi-proxy upstream pool selection, failover retry, and request optimization.

use crate::domain::{ProxySelectionQuery, UpstreamProxy};
use crate::mock_factory::MockFactory;
use crate::ports::{FlowOptimizerPort, ProxyPoolManagementPort};
use crate::request_optimizer::optimize_request_headers;
use crate::service::ProxyGuardService;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct ProxyServer {
    listener_port: u16,
    upstream_proxy: Option<String>,
    service: Arc<ProxyGuardService>,
    shutdown_flag: Arc<AtomicBool>,
}

impl ProxyServer {
    pub fn new(
        listener_port: u16,
        upstream_proxy: Option<String>,
        service: Arc<ProxyGuardService>,
    ) -> Self {
        Self {
            listener_port,
            upstream_proxy,
            service,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown_flag)
    }

    /// Spawns the TCP proxy listener in a background worker thread.
    pub fn start_background(self) -> Result<thread::JoinHandle<()>, String> {
        let addr = format!("127.0.0.1:{}", self.listener_port);
        let listener = TcpListener::bind(&addr)
            .map_err(|e| format!("Failed to bind Proxy Guard listener on {addr}: {e}"))?;

        let _ = listener.set_nonblocking(true);

        let shutdown = Arc::clone(&self.shutdown_flag);
        let service = Arc::clone(&self.service);
        let upstream = self.upstream_proxy.clone();

        let handle = thread::spawn(move || loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            match listener.accept() {
                Ok((client_stream, _)) => {
                    let svc = Arc::clone(&service);
                    let up = upstream.clone();
                    thread::spawn(move || {
                        handle_client_connection(client_stream, svc, up);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        });

        Ok(handle)
    }
}

fn handle_client_connection(
    mut client: TcpStream,
    service: Arc<ProxyGuardService>,
    fallback_upstream: Option<String>,
) {
    let _ = client.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = client.set_write_timeout(Some(Duration::from_secs(15)));

    // Accumulate header bytes until \r\n\r\n or \n\n with size limit
    let mut header_buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    let mut header_end = None;

    while header_buf.len() < 65536 {
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                header_buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_header_end(&header_buf) {
                    header_end = Some(pos);
                    break;
                }
            }
        }
    }

    let end_idx = match header_end {
        Some(idx) => idx,
        None => return,
    };

    let header_bytes = &header_buf[..end_idx];
    let body_prefix = &header_buf[end_idx..];

    let header_str = match std::str::from_utf8(header_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let first_line = match header_str.lines().next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let target = parts[1];
    let proxy_query = extract_proxy_query_from_headers(header_str);

    if method.eq_ignore_ascii_case("CONNECT") {
        handle_https_connect(
            &mut client,
            target,
            &service,
            &proxy_query,
            fallback_upstream,
        );
    } else {
        handle_http_request(
            &mut client,
            method,
            target,
            header_bytes,
            body_prefix,
            &service,
            &proxy_query,
            fallback_upstream,
        );
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len() {
        if i + 4 <= buf.len() && &buf[i..i + 4] == b"\r\n\r\n" {
            return Some(i + 4);
        }
        if i + 2 <= buf.len() && &buf[i..i + 2] == b"\n\n" {
            return Some(i + 2);
        }
    }
    None
}

fn extract_proxy_query_from_headers(header_str: &str) -> ProxySelectionQuery {
    let mut query = ProxySelectionQuery::default();
    for line in header_str.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("x-proxy-session:") {
            query.session_id = Some(val.trim().to_string());
        } else if let Some(val) = lower.strip_prefix("x-proxy-country:") {
            query.country = Some(val.trim().to_string());
        } else if let Some(val) = lower.strip_prefix("x-proxy-tag:") {
            query.tag = Some(val.trim().to_string());
        } else if let Some(val) = lower.strip_prefix("x-proxy-id:") {
            query.preferred_id = Some(val.trim().to_string());
        }
    }
    query
}

fn handle_https_connect(
    client: &mut TcpStream,
    host_port: &str,
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_upstream: Option<String>,
) {
    let fake_url = format!("https://{host_port}/");
    let decision = service.evaluate_url(&fake_url, "CONNECT", None);

    if decision.is_blocked() {
        // Drop connection immediately! Zero bytes to upstream paid proxy.
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 26\r\nConnection: close\r\n\r\nBlocked by Proxy Guard\n");
        return;
    }

    let (mut upstream, proxy_opt) =
        match dial_upstream(service, query, fallback_upstream.as_deref(), host_port) {
            Some(res) => res,
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };

    // If forwarded via upstream proxy, perform CONNECT handshake
    if let Some(ref proxy) = proxy_opt {
        let auth_hdr = match &proxy.auth {
            Some(a) => {
                let creds = format!("{}:{}", a.username, a.password);
                format!(
                    "Proxy-Authorization: Basic {}\r\n",
                    base64_encode(creds.as_bytes())
                )
            }
            None => String::new(),
        };

        let connect_req =
            format!("CONNECT {host_port} HTTP/1.1\r\nHost: {host_port}\r\n{auth_hdr}\r\n");
        if upstream.write_all(connect_req.as_bytes()).is_err() {
            service.record_proxy_result(&proxy.id, false);
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }

        let mut resp_header_buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 512];
        let mut resp_header_end = None;

        while resp_header_buf.len() < 16384 {
            match upstream.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    service.record_proxy_result(&proxy.id, false);
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                    return;
                }
                Ok(n) => {
                    resp_header_buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_header_end(&resp_header_buf) {
                        resp_header_end = Some(pos);
                        break;
                    }
                }
            }
        }

        let resp_end = match resp_header_end {
            Some(idx) => idx,
            None => {
                service.record_proxy_result(&proxy.id, false);
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };

        let header_str = String::from_utf8_lossy(&resp_header_buf[..resp_end]);
        let first_line = header_str.lines().next().unwrap_or("");
        if !first_line.contains(" 200") {
            service.record_proxy_result(&proxy.id, false);
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }

        service.record_proxy_result(&proxy.id, true);

        // Forward any leftover bytes after CONNECT response headers to client
        let leftover = &resp_header_buf[resp_end..];
        if !leftover.is_empty() && client.write_all(leftover).is_err() {
            return;
        }
    }

    let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");

    // Clear initial request timeouts for the established tunnel
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);
    let _ = upstream.set_read_timeout(None);
    let _ = upstream.set_write_timeout(None);

    bidirectional_splice(client, &mut upstream);
}

#[allow(clippy::too_many_arguments)]
fn handle_http_request(
    client: &mut TcpStream,
    method: &str,
    target: &str,
    header_bytes: &[u8],
    body_prefix: &[u8],
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_upstream: Option<String>,
) {
    let decision = service.evaluate_url(target, method, None);

    if decision.is_blocked() {
        if let Some(mock) = decision.mock {
            if mock.content_type.starts_with("image/gif") {
                let gif_bytes = MockFactory::TRANSPARENT_GIF_BYTES;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    gif_bytes.len()
                );
                let _ = client.write_all(resp.as_bytes());
                let _ = client.write_all(gif_bytes);
            } else {
                let resp = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    mock.status,
                    mock.content_type,
                    mock.body.len(),
                    mock.body
                );
                let _ = client.write_all(resp.as_bytes());
            }
        } else {
            let _ = client.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        }
        return;
    }

    let target_host_port = extract_host_port(target, 80);
    let (mut upstream, proxy_opt) = match dial_upstream(
        service,
        query,
        fallback_upstream.as_deref(),
        &target_host_port,
    ) {
        Some(res) => res,
        None => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }
    };

    // Optimize headers: strip tracking fingerprinting headers and compress
    let mut optimized_headers = optimize_request_headers(header_bytes);

    // If upstream proxy has auth, inject Proxy-Authorization
    if let Some(ref proxy) = proxy_opt {
        if let Some(ref a) = proxy.auth {
            let creds = format!("{}:{}", a.username, a.password);
            let auth_line = format!(
                "Proxy-Authorization: Basic {}\r\n",
                base64_encode(creds.as_bytes())
            );
            // Insert auth line right before trailing \r\n
            if optimized_headers.ends_with(b"\r\n\r\n") {
                let insert_pos = optimized_headers.len() - 2;
                let mut new_hdr = Vec::with_capacity(optimized_headers.len() + auth_line.len());
                new_hdr.extend_from_slice(&optimized_headers[..insert_pos]);
                new_hdr.extend_from_slice(auth_line.as_bytes());
                new_hdr.extend_from_slice(b"\r\n");
                optimized_headers = new_hdr;
            }
        }
    }

    let _ = upstream.write_all(&optimized_headers);
    if !body_prefix.is_empty() {
        let _ = upstream.write_all(body_prefix);
    }

    if let Some(ref proxy) = proxy_opt {
        service.record_proxy_result(&proxy.id, true);
    }

    // Clear request reception timeouts before streaming response
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);

    bidirectional_splice(client, &mut upstream);
}

/// Connects to upstream with multi-proxy selection and automatic failover retry.
fn dial_upstream(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_url: Option<&str>,
    target_host_port: &str,
) -> Option<(TcpStream, Option<UpstreamProxy>)> {
    // 1. Try pool proxy selection
    if let Some(proxy) = service.select_proxy(query) {
        let addr = format!("{}:{}", proxy.host, proxy.port);
        if let Some(stream) = try_connect_socket(&addr) {
            return Some((stream, Some(proxy)));
        }

        // Connection failed! Record failure and try fallback proxy
        service.record_proxy_result(&proxy.id, false);
        if let Some(fallback_proxy) = service.select_fallback(&proxy.id, query) {
            let fb_addr = format!("{}:{}", fallback_proxy.host, fallback_proxy.port);
            if let Some(stream) = try_connect_socket(&fb_addr) {
                return Some((stream, Some(fallback_proxy)));
            }
            service.record_proxy_result(&fallback_proxy.id, false);
        }
    }

    // 2. Try static fallback CLI proxy if provided
    if let Some(up) = fallback_url {
        let addr = extract_host_port(up, 8080);
        if let Some(stream) = try_connect_socket(&addr) {
            let parsed = UpstreamProxy::parse(up, None, None, None).ok();
            return Some((stream, parsed));
        }
    }

    // 3. Fallback direct connection to target
    try_connect_socket(target_host_port).map(|s| (s, None))
}

fn try_connect_socket(host_port: &str) -> Option<TcpStream> {
    let addrs: Vec<_> = host_port.to_socket_addrs().ok()?.collect();
    for addr in &addrs {
        if let Ok(stream) = TcpStream::connect_timeout(addr, Duration::from_secs(15)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
            return Some(stream);
        }
    }
    None
}

fn bidirectional_splice(stream_a: &mut TcpStream, stream_b: &mut TcpStream) {
    let mut a_reader = match stream_a.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut b_writer = match stream_b.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut a_writer = match stream_a.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut b_reader = match stream_b.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    };

    let t1 = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match a_reader.read(&mut buf) {
                Ok(0) => {
                    let _ = b_writer.shutdown(Shutdown::Write);
                    break;
                }
                Err(_) => {
                    let _ = b_writer.shutdown(Shutdown::Both);
                    break;
                }
                Ok(n) => {
                    if b_writer.write_all(&buf[..n]).is_err() {
                        let _ = a_reader.shutdown(Shutdown::Both);
                        break;
                    }
                }
            }
        }
    });

    let mut buf = [0u8; 8192];
    loop {
        match b_reader.read(&mut buf) {
            Ok(0) => {
                let _ = a_writer.shutdown(Shutdown::Write);
                break;
            }
            Err(_) => {
                let _ = a_writer.shutdown(Shutdown::Both);
                break;
            }
            Ok(n) => {
                if a_writer.write_all(&buf[..n]).is_err() {
                    let _ = b_reader.shutdown(Shutdown::Both);
                    break;
                }
            }
        }
    }

    let _ = t1.join();
}

fn extract_host_port(url_or_addr: &str, default_port: u16) -> String {
    let clean = if let Some(idx) = url_or_addr.find("://") {
        &url_or_addr[idx + 3..]
    } else {
        url_or_addr
    };

    let host_part = clean.split('/').next().unwrap_or(clean);
    if host_part.contains(':') {
        host_part.to_string()
    } else {
        format!("{host_part}:{default_port}")
    }
}

fn base64_encode(input: &[u8]) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(CHARSET[(b0 >> 2) as usize] as char);
        out.push(CHARSET[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARSET[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARSET[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
