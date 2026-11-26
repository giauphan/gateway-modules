//! HTTPS CONNECT Tunneling and Failover Splicing.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

use super::utils::{
    base64_encode, find_header_end, perform_socks5_connect, split_host_port, try_connect_socket,
    try_connect_target_socket,
};
use crate::domain::{ProxyProtocol, ProxySelectionQuery, RuleAction, UpstreamProxy};
use crate::ports::{FlowOptimizerPort, ProxyPoolManagementPort};
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

    let (mut upstream, leftover) = if decision.action == RuleAction::BypassDirect {
        match try_connect_target_socket(host_port) {
            Some(s) => (s, Vec::new()),
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        }
    } else {
        match dial_connect_tunnel(service, query, fallback_upstream.as_deref(), host_port) {
            Some(res) => res,
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
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

    // Forward buffered leftover bytes from upstream handshake if any
    if !leftover.is_empty() && client.write_all(&leftover).is_err() {
        return;
    }

    // Forward pipelined client data (e.g. TLS ClientHello) to upstream
    if !client_prefix.is_empty() && upstream.write_all(client_prefix).is_err() {
        return;
    }

    // Set a reasonable idle timeout (120s) to prevent permanent thread/socket leaks
    let idle_timeout = Some(Duration::from_secs(120));
    let _ = client.set_read_timeout(idle_timeout);
    let _ = client.set_write_timeout(idle_timeout);
    let _ = upstream.set_read_timeout(idle_timeout);
    let _ = upstream.set_write_timeout(idle_timeout);

    bidirectional_splice(client, &mut upstream);
}

fn perform_connect_handshake(
    mut stream: TcpStream,
    host_port: &str,
    auth: Option<&crate::domain::ProxyAuth>,
) -> Result<(TcpStream, Vec<u8>), ()> {
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
        return Err(());
    }

    let mut resp_header_buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 512];
    let mut resp_header_end = None;

    while resp_header_buf.len() < 16384 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(()),
            Ok(n) => {
                resp_header_buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_header_end(&resp_header_buf) {
                    resp_header_end = Some(pos);
                    break;
                }
            }
        }
    }

    let resp_end = resp_header_end.ok_or(())?;
    let header_str = String::from_utf8_lossy(&resp_header_buf[..resp_end]);
    let first_line = header_str.lines().next().unwrap_or("");
    if !first_line.contains(" 200") {
        return Err(());
    }

    let leftover = resp_header_buf[resp_end..].to_vec();
    Ok((stream, leftover))
}

fn try_dial_connect_proxy(
    proxy: &UpstreamProxy,
    host_port: &str,
) -> Result<(TcpStream, Vec<u8>), ()> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let stream = try_connect_socket(&addr).ok_or(())?;
    if proxy.protocol == ProxyProtocol::Socks5 {
        let (th, port_num) = split_host_port(host_port, 443);
        perform_socks5_connect(stream, th, port_num, proxy.auth.as_ref())
            .map(|s| (s, Vec::new()))
            .ok_or(())
    } else {
        perform_connect_handshake(stream, host_port, proxy.auth.as_ref())
    }
}

fn dial_connect_tunnel(
    service: &ProxyGuardService,
    query: &ProxySelectionQuery,
    fallback_url: Option<&str>,
    host_port: &str,
) -> Option<(TcpStream, Vec<u8>)> {
    // 1. Try pool proxy selection
    if let Some(proxy) = service.select_proxy(query) {
        match try_dial_connect_proxy(&proxy, host_port) {
            Ok((stream, leftover)) => {
                service.record_proxy_result(&proxy.id, true);
                return Some((stream, leftover));
            }
            Err(()) => {
                service.record_proxy_result(&proxy.id, false);
                if let Some(fallback_proxy) = service.select_fallback(&proxy.id, query) {
                    if let Ok((stream, leftover)) =
                        try_dial_connect_proxy(&fallback_proxy, host_port)
                    {
                        service.record_proxy_result(&fallback_proxy.id, true);
                        return Some((stream, leftover));
                    }
                    service.record_proxy_result(&fallback_proxy.id, false);
                }
            }
        }
    }

    // 2. Try static fallback CLI proxy if provided
    if let Some(up) = fallback_url {
        if let Ok(parsed) = UpstreamProxy::parse(up, None, None, None) {
            if let Ok((s, leftover)) = try_dial_connect_proxy(&parsed, host_port) {
                return Some((s, leftover));
            }
        }
    }

    // 3. Direct connection to target with SSRF protection
    try_connect_target_socket(host_port).map(|s| (s, Vec::new()))
}

pub fn bidirectional_splice(stream_a: &mut TcpStream, stream_b: &mut TcpStream) {
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
            }
        }
    }

    let _ = stream_a.shutdown(Shutdown::Both);
    let _ = stream_b.shutdown(Shutdown::Both);
    let _ = t1.join();
}
