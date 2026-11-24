//! Utility functions for HTTP header inspection, socket dialing, and encoding.
//!
//! Clean Architecture: Infrastructure Adapter Layer.

use crate::domain::ProxySelectionQuery;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
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

pub fn extract_proxy_query_from_headers(header_str: &str) -> ProxySelectionQuery {
    let mut query = ProxySelectionQuery::default();
    for line in header_str.lines() {
        if let Some((name, val)) = line.split_once(':') {
            let name_lower = name.trim().to_ascii_lowercase();
            let val_trimmed = val.trim();
            match name_lower.as_str() {
                "x-proxy-session" => query.session_id = Some(val_trimmed.to_string()),
                "x-proxy-country" => query.country = Some(val_trimmed.to_string()),
                "x-proxy-tag" => query.tag = Some(val_trimmed.to_string()),
                "x-proxy-id" => query.preferred_id = Some(val_trimmed.to_string()),
                _ => {}
            }
        }
    }
    query
}

pub fn extract_host_port(url_or_addr: &str, default_port: u16) -> String {
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

pub fn parse_content_length(header_bytes: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(header_bytes).ok()?;
    for line in s.lines() {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                return val.trim().parse::<usize>().ok();
            }
        }
    }
    None
}

pub fn try_connect_socket(host_port: &str) -> Option<TcpStream> {
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

pub fn base64_encode(input: &[u8]) -> String {
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
