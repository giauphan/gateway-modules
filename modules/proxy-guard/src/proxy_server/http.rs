//! HTTP Request Handler and Upstream Forwarding.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

use super::utils::{
    base64_encode, extract_host_port, is_chunked_transfer_encoding, parse_content_length,
    perform_socks5_connect, try_connect_socket, try_connect_target_socket,
};
use crate::domain::{ProxyProtocol, ProxySelectionQuery, UpstreamProxy};
use crate::mock_factory::MockFactory;
use crate::ports::{FlowOptimizerPort, ProxyPoolManagementPort};
use crate::request_optimizer::optimize_request_headers;
use crate::service::ProxyGuardService;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

#[allow(clippy::too_many_arguments)]
pub fn handle_http_request(
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

    // If upstream proxy has auth and is HTTP, inject Proxy-Authorization
    if let Some(ref proxy) = proxy_opt {
        if proxy.protocol == ProxyProtocol::Http {
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
    }

    // Write headers to upstream with error accounting
    if upstream.write_all(&optimized_headers).is_err() {
        if let Some(ref proxy) = proxy_opt {
            service.record_proxy_result(&proxy.id, false);
        }
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    }

    // Forward the evaluated body prefix
    if !body_prefix.is_empty() && upstream.write_all(body_prefix).is_err() {
        if let Some(ref proxy) = proxy_opt {
            service.record_proxy_result(&proxy.id, false);
        }
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    }

    // Forward request body according to Content-Length or Transfer-Encoding: chunked
    let content_len = parse_content_length(header_bytes).unwrap_or(0);
    if is_chunked_transfer_encoding(header_bytes) {
        let mut body_tracker = body_prefix.to_vec();
        let mut chunk_buf = [0u8; 4096];
        while !body_tracker.windows(5).any(|w| w == b"0\r\n\r\n")
            && !body_tracker.ends_with(b"0\r\n\r\n")
        {
            match client.read(&mut chunk_buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if upstream.write_all(&chunk_buf[..n]).is_err() {
                        if let Some(ref proxy) = proxy_opt {
                            service.record_proxy_result(&proxy.id, false);
                        }
                        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                        return;
                    }
                    body_tracker.extend_from_slice(&chunk_buf[..n]);
                }
            }
        }
    } else if body_prefix.len() < content_len {
        let mut remaining = content_len - body_prefix.len();
        let mut buf = [0u8; 8192];
        while remaining > 0 {
            let to_read = remaining.min(buf.len());
            match client.read(&mut buf[..to_read]) {
                Ok(0) | Err(_) => {
                    // Client disconnected early; do not misattribute to proxy failure
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                    return;
                }
                Ok(n) => {
                    if upstream.write_all(&buf[..n]).is_err() {
                        if let Some(ref proxy) = proxy_opt {
                            service.record_proxy_result(&proxy.id, false);
                        }
                        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                        return;
                    }
                    remaining -= n;
                }
            }
        }
    }

    if let Some(ref proxy) = proxy_opt {
        service.record_proxy_result(&proxy.id, true);
    }

    // Enforce HTTP request boundary: stop reading further requests from client
    let _ = client.shutdown(Shutdown::Read);
    let _ = upstream.shutdown(Shutdown::Write);

    // Stream upstream response back to client until EOF
    let mut resp_buf = [0u8; 8192];
    loop {
        match upstream.read(&mut resp_buf) {
            Ok(0) => break,
            Ok(n) => {
                if client.write_all(&resp_buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = client.flush();
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
            if proxy.protocol == ProxyProtocol::Socks5 {
                let (th, tp) = target_host_port
                    .split_once(':')
                    .unwrap_or((target_host_port, "80"));
                let port_num = tp.parse::<u16>().unwrap_or(80);
                if let Some(socks_stream) =
                    perform_socks5_connect(stream, th, port_num, proxy.auth.as_ref())
                {
                    return Some((socks_stream, Some(proxy)));
                }
            } else {
                return Some((stream, Some(proxy)));
            }
        }

        // Connection failed! Record failure and try fallback proxy
        service.record_proxy_result(&proxy.id, false);
        if let Some(fallback_proxy) = service.select_fallback(&proxy.id, query) {
            let fb_addr = format!("{}:{}", fallback_proxy.host, fallback_proxy.port);
            if let Some(stream) = try_connect_socket(&fb_addr) {
                if fallback_proxy.protocol == ProxyProtocol::Socks5 {
                    let (th, tp) = target_host_port
                        .split_once(':')
                        .unwrap_or((target_host_port, "80"));
                    let port_num = tp.parse::<u16>().unwrap_or(80);
                    if let Some(socks_stream) =
                        perform_socks5_connect(stream, th, port_num, fallback_proxy.auth.as_ref())
                    {
                        return Some((socks_stream, Some(fallback_proxy)));
                    }
                } else {
                    return Some((stream, Some(fallback_proxy)));
                }
            }
            service.record_proxy_result(&fallback_proxy.id, false);
        }
    }

    // 2. Try static fallback CLI proxy if provided
    if let Some(up) = fallback_url {
        let parsed = UpstreamProxy::parse(up, None, None, None).ok();
        let addr = extract_host_port(up, 8080);
        let (th, tp) = target_host_port
            .split_once(':')
            .unwrap_or((target_host_port, "80"));
        let port_num = tp.parse::<u16>().unwrap_or(80);

        if let Some(stream) = try_connect_socket(&addr) {
            let is_socks5 = parsed
                .as_ref()
                .is_some_and(|p| p.protocol == ProxyProtocol::Socks5);
            if is_socks5 {
                let auth = parsed.as_ref().and_then(|p| p.auth.as_ref());
                if let Some(socks_stream) = perform_socks5_connect(stream, th, port_num, auth) {
                    return Some((socks_stream, parsed));
                }
            } else {
                return Some((stream, parsed));
            }
        }
    }

    // 3. Fallback direct connection to target with strict SSRF protection
    try_connect_target_socket(target_host_port).map(|s| (s, None))
}
