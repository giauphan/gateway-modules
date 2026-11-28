//! HTTPS CONNECT Tunneling and Failover Splicing.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

use super::utils::{
    base64_encode, find_header_end, perform_socks5_connect, split_host_port, try_connect_socket,
    try_connect_target_socket,
};
use crate::domain::{ProxyProtocol, ProxySelectionQuery, RuleAction, UpstreamProxy};
use crate::ports::{FlowOptimizerPort, MetricsPort, ProxyPoolManagementPort};
use crate::service::ProxyGuardService;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;
use std::time::Duration;

pub fn handle_https_connect(
    client: &mut TcpStream,
    host_port: &str,
    client_prefix: &[u8],
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

    let (mut upstream, leftover, pool_label): (TcpStream, Vec<u8>, String) = match &decision.action
    {
        RuleAction::BypassDirect => match try_connect_target_socket(host_port) {
            Some(s) => (s, Vec::new(), "direct".to_string()),
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        },
        RuleAction::RoutePool(pool_name) => {
            let mut pool_query = query.clone();
            pool_query.pool = Some(pool_name.clone());
            match dial_named_pool_connect_tunnel(service, &pool_query, host_port) {
                Some((s, l)) => (s, l, pool_name.clone()),
                None => {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                    return;
                }
            }
        }
        _ => {
            let label = query.pool.clone().unwrap_or_else(|| "default".to_string());
            match dial_connect_tunnel(service, query, fallback_upstream.as_deref(), host_port) {
                Some((s, l)) => (s, l, label),
                None => {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                    return;
                }
            }
        }
    };

    // RFC 9110: Write complete success response to client FIRST
    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .is_err()
    {
        return;
    }

    let mut total_tx = 0u64;
    let mut total_rx = 0u64;

    // Forward buffered leftover bytes from upstream handshake if any
    if !leftover.is_empty() {
        if client.write_all(&leftover).is_err() {
            return;
        }
        total_rx = total_rx.saturating_add(leftover.len() as u64);
    }

    // Forward pipelined client data (e.g. TLS ClientHello) to upstream
    if !client_prefix.is_empty() {
        if upstream.write_all(client_prefix).is_err() {
            return;
        }
        total_tx = total_tx.saturating_add(client_prefix.len() as u64);
    }

    // Set a reasonable idle timeout (120s) to prevent permanent thread/socket leaks
    let idle_timeout = Some(Duration::from_secs(120));
    let _ = client.set_read_timeout(idle_timeout);
    let _ = client.set_write_timeout(idle_timeout);
    let _ = upstream.set_read_timeout(idle_timeout);
    let _ = upstream.set_write_timeout(idle_timeout);

    let (spliced_tx, spliced_rx) = bidirectional_splice(client, &mut upstream);
    total_tx = total_tx.saturating_add(spliced_tx);
    total_rx = total_rx.saturating_add(spliced_rx);
    service.record_upstream_transfer(&pool_label, total_tx, total_rx);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectError {
    Auth407,
    Other,
}

const MAX_CONNECT_FAILOVER_ATTEMPTS: usize = 5;

fn perform_connect_handshake(
    mut stream: TcpStream,
    host_port: &str,
    auth: Option<&crate::domain::ProxyAuth>,
) -> Result<(TcpStream, Vec<u8>), ConnectError> {
    let auth_hdr = match auth {
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
    if stream.write_all(connect_req.as_bytes()).is_err() {
        return Err(ConnectError::Other);
    }

    let mut resp_header_buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 512];
    let mut resp_header_end = None;

    while resp_header_buf.len() < 16384 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(ConnectError::Other),
            Ok(n) => {
                resp_header_buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_header_end(&resp_header_buf) {
                    resp_header_end = Some(pos);
                    break;
                }
            }
        }
    }

    let resp_end = resp_header_end.ok_or(ConnectError::Other)?;
    let header_str = String::from_utf8_lossy(&resp_header_buf[..resp_end]);
    let first_line = header_str.lines().next().unwrap_or("");
    if first_line.contains(" 407") {
        return Err(ConnectError::Auth407);
    }
    if !first_line.contains(" 200") {
        return Err(ConnectError::Other);
    }

    let leftover = resp_header_buf[resp_end..].to_vec();
    Ok((stream, leftover))
}

fn try_dial_connect_proxy(
    proxy: &UpstreamProxy,
    host_port: &str,
) -> Result<(TcpStream, Vec<u8>), ConnectError> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let stream = try_connect_socket(&addr).ok_or(ConnectError::Other)?;
    if proxy.protocol == ProxyProtocol::Socks5 {
        let (th, port_num) = split_host_port(host_port, 443);
        perform_socks5_connect(stream, th, port_num, proxy.auth.as_ref())
            .map(|s| (s, Vec::new()))
            .ok_or(ConnectError::Other)
    } else {
        perform_connect_handshake(stream, host_port, proxy.auth.as_ref())
    }
}

fn record_connect_failure(service: &ProxyGuardService, proxy_id: &str, err: ConnectError) {
    match err {
        ConnectError::Auth407 => service.record_proxy_auth_failure(proxy_id),
        ConnectError::Other => service.record_proxy_result(proxy_id, false),
    }
}

/// Dials strictly through the requested named pool with multi-hop failover (fail-closed: never falls back to direct).
fn dial_named_pool_connect_tunnel(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    host_port: &str,
) -> Option<(TcpStream, Vec<u8>)> {
    let mut attempt_query = query.clone();
    let mut current = service.select_proxy(&attempt_query)?;

    for _ in 0..MAX_CONNECT_FAILOVER_ATTEMPTS {
        match try_dial_connect_proxy(&current, host_port) {
            Ok((stream, leftover)) => {
                service.record_proxy_result(&current.id, true);
                return Some((stream, leftover));
            }
            Err(err) => {
                record_connect_failure(service, &current.id, err);
                attempt_query.exclude_ids.push(current.id.clone());
                current = service.select_fallback(&current.id, &attempt_query)?;
            }
        }
    }
    None
}

fn dial_connect_tunnel(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_url: Option<&str>,
    host_port: &str,
) -> Option<(TcpStream, Vec<u8>)> {
    // 1. Try pool proxy selection with multi-hop failover across all remaining healthy proxies
    let mut attempt_query = query.clone();
    if let Some(mut current) = service.select_proxy(&attempt_query) {
        for _ in 0..MAX_CONNECT_FAILOVER_ATTEMPTS {
            match try_dial_connect_proxy(&current, host_port) {
                Ok((stream, leftover)) => {
                    service.record_proxy_result(&current.id, true);
                    return Some((stream, leftover));
                }
                Err(err) => {
                    record_connect_failure(service, &current.id, err);
                    attempt_query.exclude_ids.push(current.id.clone());
                    match service.select_fallback(&current.id, &attempt_query) {
                        Some(next_proxy) => current = next_proxy,
                        None => break,
                    }
                }
            }
        }
    }

    // 2. Try static fallback CLI proxy ONLY when no pool proxies are configured
    if service.list_proxies().is_empty() {
        if let Some(up) = fallback_url {
            if let Ok(parsed) = UpstreamProxy::parse(up, None, None, None) {
                if let Ok((s, leftover)) = try_dial_connect_proxy(&parsed, host_port) {
                    return Some((s, leftover));
                }
            }
        }
    }

    // 3. Direct connection to target with SSRF protection (only when pool is empty)
    if service.list_proxies().is_empty() {
        return try_connect_target_socket(host_port).map(|s| (s, Vec::new()));
    }
    None
}

/// Splices two TCP streams bidirectionally without TLS termination, returning `(bytes_a_to_b, bytes_b_to_a)`.
pub fn bidirectional_splice(stream_a: &mut TcpStream, stream_b: &mut TcpStream) -> (u64, u64) {
    let mut a_reader = match stream_a.try_clone() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let mut b_writer = match stream_b.try_clone() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let mut a_writer = match stream_a.try_clone() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };
    let mut b_reader = match stream_b.try_clone() {
        Ok(c) => c,
        Err(_) => return (0, 0),
    };

    let t1 = thread::spawn(move || -> u64 {
        let mut buf = [0u8; 8192];
        let mut tx_bytes = 0u64;
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
                    tx_bytes = tx_bytes.saturating_add(n as u64);
                }
            }
        }
        tx_bytes
    });

    let mut buf = [0u8; 8192];
    let mut rx_bytes = 0u64;
    loop {
        match b_reader.read(&mut buf) {
            Ok(0) => {
                let _ = a_writer.flush();
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
                rx_bytes = rx_bytes.saturating_add(n as u64);
            }
        }
    }

    let _ = stream_a.shutdown(Shutdown::Both);
    let _ = stream_b.shutdown(Shutdown::Both);
    let tx_bytes = t1.join().unwrap_or(0);
    (tx_bytes, rx_bytes)
}
