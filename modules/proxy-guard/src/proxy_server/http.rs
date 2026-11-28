//! HTTP Request Handler and Upstream Forwarding.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

use super::utils::{
    base64_encode, extract_host_port, is_chunked_transfer_encoding, parse_content_length,
    perform_socks5_connect, split_host_port, try_connect_socket, try_connect_target_socket,
};
use crate::domain::{
    FilterDecision, ProxyProtocol, ProxySelectionQuery, RuleAction, UpstreamProxy,
};
use crate::http_cache::{is_not_modified_response, CacheLookup, HttpResponseCache};
use crate::mock_factory::MockFactory;
use crate::ports::{FlowOptimizerPort, MetricsPort, ProxyPoolManagementPort};
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

fn ends_with_chunk_terminator(tail: &[u8], len: usize) -> bool {
    len >= 5 && &tail[len - 5..len] == b"0\r\n\r\n"
}

fn forward_chunked_body(
    client: &mut TcpStream,
    upstream: &mut TcpStream,
    body_prefix: &[u8],
) -> Result<(), ()> {
    // Keep at most the last 4 bytes of previous reads to detect `0\r\n\r\n` across boundaries in O(1) space.
    let mut tail = [0u8; 4];
    let mut tail_len = 0usize;

    let check_and_advance = |slice: &[u8], tail: &mut [u8; 4], tail_len: &mut usize| -> bool {
        let mut combined = [0u8; 4 + 4096];
        let mut idx = 0;
        if *tail_len > 0 {
            combined[..*tail_len].copy_from_slice(&tail[..*tail_len]);
            idx = *tail_len;
        }
        // Scan in chunks if slice > 4096, though here slice is <= 4096 on reads (and body_prefix is < 64KB)
        for chunk in slice.chunks(4096) {
            let total = idx + chunk.len();
            combined[idx..total].copy_from_slice(chunk);
            if combined[..total].windows(5).any(|w| w == b"0\r\n\r\n") {
                return true;
            }
            let keep = total.min(4);
            tail[..keep].copy_from_slice(&combined[total - keep..total]);
            *tail_len = keep;
            combined[..keep].copy_from_slice(&tail[..keep]);
            idx = keep;
        }
        ends_with_chunk_terminator(&combined, idx)
    };

    if check_and_advance(body_prefix, &mut tail, &mut tail_len) {
        return Ok(());
    }

    let mut chunk_buf = [0u8; 4096];
    loop {
        match client.read(&mut chunk_buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if upstream.write_all(&chunk_buf[..n]).is_err() {
                    return Err(());
                }
                if check_and_advance(&chunk_buf[..n], &mut tail, &mut tail_len) {
                    break;
                }
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

fn inject_header_before_end(mut headers: Vec<u8>, header_line: &str) -> Vec<u8> {
    if headers.ends_with(b"\r\n\r\n") {
        let insert_pos = headers.len() - 2;
        let mut new_hdr = Vec::with_capacity(headers.len() + header_line.len());
        new_hdr.extend_from_slice(&headers[..insert_pos]);
        new_hdr.extend_from_slice(header_line.as_bytes());
        new_hdr.extend_from_slice(b"\r\n");
        headers = new_hdr;
    }
    headers
}

fn is_http_407_prefix(chunk: &[u8]) -> bool {
    let head_len = chunk.len().min(64);
    if let Ok(s) = std::str::from_utf8(&chunk[..head_len]) {
        if let Some(first_line) = s.lines().next() {
            return first_line.starts_with("HTTP/") && first_line.contains(" 407");
        }
    }
    false
}

fn pipe_response(
    upstream: &mut TcpStream,
    client: &mut TcpStream,
    max_capture_bytes: usize,
) -> (u64, Option<Vec<u8>>, bool) {
    let mut resp_buf = [0u8; 8192];
    let mut total_rx = 0u64;
    let mut saw_407 = false;
    let mut captured = if max_capture_bytes > 0 {
        Some(Vec::with_capacity(max_capture_bytes.min(65536)))
    } else {
        None
    };

    loop {
        match upstream.read(&mut resp_buf) {
            Ok(0) => break,
            Ok(n) => {
                if total_rx == 0 && is_http_407_prefix(&resp_buf[..n]) {
                    saw_407 = true;
                }
                total_rx = total_rx.saturating_add(n as u64);
                if let Some(ref mut buf) = captured {
                    if buf.len().saturating_add(n) <= max_capture_bytes {
                        buf.extend_from_slice(&resp_buf[..n]);
                    } else {
                        captured = None;
                    }
                }
                if client.write_all(&resp_buf[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = client.flush();
    (total_rx, captured, saw_407)
}

fn read_bounded_response(upstream: &mut TcpStream, max_bytes: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    loop {
        match upstream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len().saturating_add(n) > max_bytes {
                    return None;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
    Some(buf)
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

    // Safe HTTP Response Cache (opt-in, strictly no MITM, bypasses auth/write/sensitive paths)
    let cache = service.http_cache();
    let cacheable = cache.is_request_cacheable(method, target, header_bytes);
    let cache_key = if cacheable {
        Some(HttpResponseCache::cache_key(method, target))
    } else {
        None
    };
    let mut revalidate_state: Option<(String, Vec<u8>)> = None;

    if let Some(ref key) = cache_key {
        match cache.lookup(key) {
            CacheLookup::Hit(cached) => {
                let _ = client.write_all(&cached.raw_response);
                let _ = client.flush();
                service.record_cache_hit(cached.raw_response.len() as u64);
                return;
            }
            CacheLookup::Revalidate { etag, entry } => {
                revalidate_state = Some((etag, entry.raw_response));
            }
            CacheLookup::Miss => {
                service.record_cache_miss();
            }
        }
    }

    let target_host_port = extract_host_port(target, 80);
    let (mut upstream, proxy_opt, pool_label): (TcpStream, Option<UpstreamProxy>, String) =
        match &decision.action {
            RuleAction::BypassDirect => match try_connect_target_socket(&target_host_port) {
                Some(s) => (s, None, "direct".to_string()),
                None => {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                    return;
                }
            },
            RuleAction::RoutePool(pool_name) => {
                let mut pool_query = query.clone();
                pool_query.pool = Some(pool_name.clone());
                match dial_named_pool_upstream(service, &pool_query, &target_host_port) {
                    Some((s, p)) => (s, Some(p), pool_name.clone()),
                    None => {
                        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                        return;
                    }
                }
            }
            _ => {
                let label = query.pool.clone().unwrap_or_else(|| "default".to_string());
                match dial_upstream(
                    service,
                    query,
                    fallback_upstream.as_deref(),
                    &target_host_port,
                ) {
                    Some((s, p)) => (s, p, label),
                    None => {
                        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                        return;
                    }
                }
            }
        };

    let mut optimized_headers = inject_proxy_auth(header_bytes, proxy_opt.as_ref());
    if let Some((ref etag, _)) = revalidate_state {
        optimized_headers =
            inject_header_before_end(optimized_headers, &format!("If-None-Match: {etag}\r\n"));
    }

    let mut bytes_tx = optimized_headers.len() as u64;
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
    bytes_tx = bytes_tx.saturating_add(body_prefix.len() as u64);

    if let Some(ref proxy) = proxy_opt {
        service.record_proxy_result(&proxy.id, true);
    }

    let _ = client.shutdown(Shutdown::Read);
    let _ = upstream.shutdown(Shutdown::Write);

    let cache_cfg = cache.config();
    let max_capture = if cacheable {
        cache_cfg.max_body_bytes.saturating_add(16_384)
    } else {
        0
    };

    if let (Some(key), Some((_, cached_raw))) = (&cache_key, revalidate_state) {
        if let Some(resp_bytes) = read_bounded_response(&mut upstream, max_capture.max(65536)) {
            let rx = resp_bytes.len() as u64;
            service.record_upstream_transfer(&pool_label, bytes_tx, rx);
            if is_not_modified_response(&resp_bytes) {
                let served = cache
                    .refresh_not_modified(key, target)
                    .map(|e| e.raw_response)
                    .unwrap_or(cached_raw);
                let _ = client.write_all(&served);
                let _ = client.flush();
                service.record_cache_hit(served.len() as u64);
                return;
            }
            cache.store(key, target, &resp_bytes);
            let _ = client.write_all(&resp_bytes);
            let _ = client.flush();
            return;
        }
    }

    let (bytes_rx, captured_opt, saw_407) = pipe_response(&mut upstream, client, max_capture);
    if saw_407 {
        if let Some(ref proxy) = proxy_opt {
            service.record_proxy_auth_failure(&proxy.id);
        }
    }
    service.record_upstream_transfer(&pool_label, bytes_tx, bytes_rx);
    if !saw_407 {
        if let (Some(key), Some(captured)) = (cache_key, captured_opt) {
            cache.store(&key, target, &captured);
        }
    }
}

/// Dials strictly from the requested named pool (fail-closed).
fn dial_named_pool_upstream(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    target_host_port: &str,
) -> Option<(TcpStream, UpstreamProxy)> {
    let proxy = service.select_proxy(query)?;
    if let Some(stream) = try_dial_proxy(&proxy, target_host_port) {
        return Some((stream, proxy));
    }
    service.record_proxy_result(&proxy.id, false);
    if let Some(fallback_proxy) = service.select_fallback(&proxy.id, query) {
        if let Some(stream) = try_dial_proxy(&fallback_proxy, target_host_port) {
            return Some((stream, fallback_proxy));
        }
        service.record_proxy_result(&fallback_proxy.id, false);
    }
    None
}

fn try_dial_proxy(proxy: &UpstreamProxy, target_host_port: &str) -> Option<TcpStream> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let stream = try_connect_socket(&addr)?;
    if proxy.protocol == ProxyProtocol::Socks5 {
        let (th, port_num) = split_host_port(target_host_port, 80);
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
