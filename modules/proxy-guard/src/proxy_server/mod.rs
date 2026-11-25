//! Native Intercepting Proxy Server.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Runs a local HTTP/CONNECT proxy that intercepts traffic before it reaches paid upstream proxies.
//! Implements multi-proxy upstream pool selection, failover retry, and request optimization.

pub mod http;
pub mod tunnel;
pub mod utils;

use http::handle_http_request;
use tunnel::handle_https_connect;
use utils::{extract_host_from_headers, extract_proxy_query_from_headers, find_header_end};

use crate::service::ProxyGuardService;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MAX_ACTIVE_CONNECTIONS: usize = 256;

pub struct ProxyServer {
    listener_port: u16,
    upstream_proxy: Option<String>,
    service: Arc<ProxyGuardService>,
    shutdown_flag: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
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
            active_connections: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown_flag)
    }

    /// Spawns the TCP proxy listener in a background worker thread.
    /// Returns the join handle and the actual bound port (useful when port 0 is used).
    pub fn start_background(self) -> Result<(thread::JoinHandle<()>, u16), String> {
        let addr = format!("0.0.0.0:{}", self.listener_port);
        let listener = TcpListener::bind(&addr)
            .or_else(|_| TcpListener::bind(format!("127.0.0.1:{}", self.listener_port)))
            .map_err(|e| {
                format!(
                    "Failed to bind Proxy Guard listener on port {}: {e}",
                    self.listener_port
                )
            })?;
        let bound_port = listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(self.listener_port);

        let _ = listener.set_nonblocking(true);

        let shutdown = Arc::clone(&self.shutdown_flag);
        let service = Arc::clone(&self.service);
        let upstream = self.upstream_proxy.clone();
        let active_conns = Arc::clone(&self.active_connections);

        let handle = thread::spawn(move || loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            match listener.accept() {
                Ok((client_stream, _)) => {
                    let current = active_conns.load(Ordering::Relaxed);
                    if current >= MAX_ACTIVE_CONNECTIONS {
                        // Throttled: drop socket to prevent thread exhaustion
                        continue;
                    }
                    active_conns.fetch_add(1, Ordering::Relaxed);

                    let svc = Arc::clone(&service);
                    let up = upstream.clone();
                    let conns_counter = Arc::clone(&active_conns);
                    thread::spawn(move || {
                        struct ConnGuard(Arc<AtomicUsize>);
                        impl Drop for ConnGuard {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::Relaxed);
                            }
                        }
                        let _guard = ConnGuard(conns_counter);
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

        Ok((handle, bound_port))
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
    let raw_target = parts[1];
    let proxy_query = extract_proxy_query_from_headers(header_str);

    // If origin-form request target (e.g. GET /index.html), reconstruct absolute target from Host header
    let target = if raw_target.starts_with('/') {
        if let Some(host) = extract_host_from_headers(header_bytes) {
            format!("http://{host}{raw_target}")
        } else {
            raw_target.to_string()
        }
    } else {
        raw_target.to_string()
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        handle_https_connect(
            &mut client,
            &target,
            body_prefix,
            &service,
            &proxy_query,
            fallback_upstream,
        );
    } else {
        handle_http_request(
            &mut client,
            method,
            &target,
            header_bytes,
            body_prefix,
            &service,
            &proxy_query,
            fallback_upstream,
        );
    }
}
