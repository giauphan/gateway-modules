//! Native Intercepting Proxy Server.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Runs a local HTTP/CONNECT proxy that intercepts traffic before it reaches paid upstream proxies.

use crate::mock_factory::MockFactory;
use crate::ports::FlowOptimizerPort;
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
    upstream_proxy: Option<String>,
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

    if method.eq_ignore_ascii_case("CONNECT") {
        handle_https_connect(&mut client, target, &service, upstream_proxy);
    } else {
        handle_http_request(
            &mut client,
            method,
            target,
            header_bytes,
            body_prefix,
            &service,
            upstream_proxy,
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

fn handle_https_connect(
    client: &mut TcpStream,
    host_port: &str,
    service: &ProxyGuardService,
    upstream_proxy: Option<String>,
) {
    let fake_url = format!("https://{host_port}/");
    let decision = service.evaluate_url(&fake_url, "CONNECT", None);

    if decision.is_blocked() {
        // Drop connection immediately! Zero bytes to upstream paid proxy.
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 26\r\nConnection: close\r\n\r\nBlocked by Proxy Guard\n");
        return;
    }

    let mut upstream = if let Some(ref up) = upstream_proxy {
        let upstream_addr = extract_host_port(up, 8080);
        let addrs: Vec<_> = match upstream_addr.to_socket_addrs() {
            Ok(iter) => iter.collect(),
            Err(_) => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };

        let mut stream_opt = None;
        for addr in &addrs {
            if let Ok(s) = TcpStream::connect_timeout(addr, Duration::from_secs(15)) {
                stream_opt = Some(s);
                break;
            }
        }

        let mut stream = match stream_opt {
            Some(s) => s,
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };

        // Bounded deadline for upstream CONNECT handshake
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));

        // Send CONNECT upstream and validate response
        let connect_req = format!("CONNECT {host_port} HTTP/1.1\r\nHost: {host_port}\r\n\r\n");
        if stream.write_all(connect_req.as_bytes()).is_err() {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }

        let mut resp_header_buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 512];
        let mut resp_header_end = None;

        while resp_header_buf.len() < 16384 {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => {
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
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };

        let header_str = String::from_utf8_lossy(&resp_header_buf[..resp_end]);
        let first_line = header_str.lines().next().unwrap_or("");
        if !first_line.contains(" 200") {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }

        // Forward any leftover bytes after CONNECT response headers to client
        let leftover = &resp_header_buf[resp_end..];
        if !leftover.is_empty() && client.write_all(leftover).is_err() {
            return;
        }

        stream
    } else {
        let addrs: Vec<_> = match host_port.to_socket_addrs() {
            Ok(iter) => iter.collect(),
            Err(_) => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        };
        let mut stream_opt = None;
        for addr in &addrs {
            if let Ok(s) = TcpStream::connect_timeout(addr, Duration::from_secs(15)) {
                stream_opt = Some(s);
                break;
            }
        }
        match stream_opt {
            Some(s) => s,
            None => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return;
            }
        }
    };

    let _ = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");

    // Clear initial request timeouts for the established tunnel
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);
    let _ = upstream.set_read_timeout(None);
    let _ = upstream.set_write_timeout(None);

    bidirectional_splice(client, &mut upstream);
}

fn handle_http_request(
    client: &mut TcpStream,
    method: &str,
    target: &str,
    header_bytes: &[u8],
    body_prefix: &[u8],
    service: &ProxyGuardService,
    upstream_proxy: Option<String>,
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

    // Forward single request with Connection: close
    let upstream_addr = match upstream_proxy {
        Some(ref up) => extract_host_port(up, 8080),
        None => extract_host_port(target, 80),
    };

    let addrs: Vec<_> = match upstream_addr.to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }
    };
    let mut stream_opt = None;
    for addr in &addrs {
        if let Ok(s) = TcpStream::connect_timeout(addr, Duration::from_secs(15)) {
            stream_opt = Some(s);
            break;
        }
    }
    let mut upstream = match stream_opt {
        Some(s) => s,
        None => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
            return;
        }
    };

    // Optimize headers: strip tracking fingerprinting headers and compress
    let optimized_headers = optimize_request_headers(header_bytes);
    let _ = upstream.write_all(&optimized_headers);
    if !body_prefix.is_empty() {
        let _ = upstream.write_all(body_prefix);
    }
    // Clear request reception timeouts before streaming response
    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);

    // Forward upstream response back to client with coordinated shutdown
    bidirectional_splice(client, &mut upstream);
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
                    // Client finished sending request (EOF). Close write half upstream, keep reading response
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
