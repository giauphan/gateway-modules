//! HTTP Request Handler and Upstream Forwarding.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

use super::utils::{
    base64_encode, extract_host_port, is_chunked_transfer_encoding, parse_content_length,
    perform_socks5_connect, try_connect_socket, try_connect_target_socket,
};
use crate::domain::{FilterDecision, ProxyProtocol, ProxySelectionQuery, UpstreamProxy};
use crate::mock_factory::MockFactory;
use crate::ports::{FlowOptimizerPort, ProxyPoolManagementPort};
use crate::request_optimizer::optimize_request_headers;
use crate::service::ProxyGuardService;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

fn send_blocked_response(client: &mut TcpStream, decision: &FilterDecision) {
    if let Some(ref mock) = decision.mock {
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
}

fn inject_proxy_auth(headers: &[u8], proxy_opt: Option<&UpstreamProxy>) -> Vec<u8> {
    let mut optimized = optimize_request_headers(headers);
    let proxy = match proxy_opt {
        Some(p) if p.protocol == ProxyProtocol::Http => p,
        _ => return optimized,
    };

    if let Some(ref a) = proxy.auth {
        let creds = format!("{}:{}", a.username, a.password);
        let auth_line = format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64_encode(creds.as_bytes())
        );
        if optimized.ends_with(b"\r\n\r\n") {
            let insert_pos = optimized.len() - 2;
            let mut new_hdr = Vec::with_capacity(optimized.len() + auth_line.len());
            new_hdr.extend_from_slice(&optimized[..insert_pos]);
            new_hdr.extend_from_slice(auth_line.as_bytes());
            new_hdr.extend_from_slice(b"\r\n");
            optimized = new_hdr;
        }
    }
    optimized
}

fn forward_chunked_body(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    body_prefix: &[u8],
) -> Result<(), ()> {
    let mut body_tracker = body_prefix.to_vec();
    let mut chunk_buf = [0u8; 4096];
    while !body_tracker.windows(5).any(|w| w == b"0\r\n\r\n")
        && !body_tracker.ends_with(b"0\r\n\r\n")
    {
        match client.read(&mut chunk_buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if upstream.write_all(&chunk_buf[..n]).is_err() {
                    return Err(());
                }
                body_tracker.extend_from_slice(&chunk_buf[..n]);
            }
        }
    }
    Ok(())
}

fn forward_content_length_body(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    remaining_bytes: usize,
) -> Result<(), ()> {
    let mut remaining = remaining_bytes;
    let mut buf = [0u8; 8192];
    while remaining > 0 {
        let to_read = remaining.min(buf.len());
        match client.read(&mut buf[..to_read]) {
            Ok(0) | Err(_) => return Err(()),
            Ok(n) => {
                if upstream.write_all(&buf[..n]).is_err() {
                    return Err(());
                }
                remaining -= n;
            }
        }
    }
    Ok(())
}

fn forward_request_body(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    header_bytes: &[u8],
    body_prefix: &[u8],
) -> Result<(), ()> {
    if !body_prefix.is_empty() && upstream.write_all(body_prefix).is_err() {
        return Err(());
    }

    let content_len = parse_content_length(header_bytes).unwrap_or(0);
    if is_chunked_transfer_encoding(header_bytes) {
        forward_chunked_body(client, upstream, body_prefix)
    } else if body_prefix.len() < content_len {
        forward_content_length_body(client, upstream, content_len - body_prefix.len())
    } else {
        Ok(())
    }
}

fn pipe_response(upstream: &mut TcpStream, client: &mut TcpStream) {
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
        send_blocked_response(client, &decision);
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

    let optimized_headers = inject_proxy_auth(header_bytes, proxy_opt.as_ref());
    if upstream.write_all(&optimized_headers).is_err() {
        if let Some(ref proxy) = proxy_opt {
            service.record_proxy_result(&proxy.id, false);
        }
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    }

    if forward_request_body(client, &mut upstream, header_bytes, body_prefix).is_err() {
        if let Some(ref proxy) = proxy_opt {
            service.record_proxy_result(&proxy.id, false);
        }
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
    }

    if let Some(ref proxy) = proxy_opt {
        service.record_proxy_result(&proxy.id, true);
    }

    let _ = client.shutdown(Shutdown::Read);
    let _ = upstream.shutdown(Shutdown::Write);
    pipe_response(&mut upstream, client);
}

fn try_dial_proxy(proxy: &UpstreamProxy, target_host_port: &str) -> Option<TcpStream> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let stream = try_connect_socket(&addr)?;
    if proxy.protocol == ProxyProtocol::Socks5 {
        let (th, tp) = target_host_port
            .split_once(':')
            .unwrap_or((target_host_port, "80"));
        let port_num = tp.parse::<u16>().unwrap_or(80);
        perform_socks5_connect(stream, th, port_num, proxy.auth.as_ref())
    } else {
        Some(stream)
    }
}

/// Connects to upstream with multi-proxy selection and automatic failover retry.
fn dial_upstream(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_url: Option<&str>,
    target_host_port: &str,
) -> Option<(TcpStream, Option<UpstreamProxy>)> {
    if let Some(proxy) = service.select_proxy(query) {
        if let Some(stream) = try_dial_proxy(&proxy, target_host_port) {
            return Some((stream, Some(proxy)));
        }

        service.record_proxy_result(&proxy.id, false);
        if let Some(fallback_proxy) = service.select_fallback(&proxy.id, query) {
            if let Some(stream) = try_dial_proxy(&fallback_proxy, target_host_port) {
                return Some((stream, Some(fallback_proxy)));
            }
            service.record_proxy_result(&fallback_proxy.id, false);
        }
    }

    if let Some(up) = fallback_url {
        if let Ok(parsed) = UpstreamProxy::parse(up, None, None, None) {
            if let Some(stream) = try_dial_proxy(&parsed, target_host_port) {
                return Some((stream, Some(parsed)));
            }
        }
    }

    try_connect_target_socket(target_host_port).map(|s| (s, None))
}
