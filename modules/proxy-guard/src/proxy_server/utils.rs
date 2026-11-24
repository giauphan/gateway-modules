//! Utility functions for HTTP header inspection, socket dialing, and encoding.
//!
//! Clean Architecture: Infrastructure Adapter Layer.

use crate::domain::{ProxyAuth, ProxySelectionQuery};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Returns true if an IP address belongs to loopback, private, link-local,
/// or cloud instance metadata ranges (SSRF protection).
pub fn is_private_or_restricted(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

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

pub fn extract_host_from_headers(header_bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(header_bytes).ok()?;
    for line in s.lines() {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("host") {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
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

pub fn is_chunked_transfer_encoding(header_bytes: &[u8]) -> bool {
    let s = match std::str::from_utf8(header_bytes) {
        Ok(v) => v,
        Err(_) => return false,
    };
    for line in s.lines() {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("transfer-encoding") {
                return val.to_ascii_lowercase().contains("chunked");
            }
        }
    }
    false
}

/// Connects to a user-configured upstream proxy socket (may be in local subnet or Docker).
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

/// Connects direct to an outbound destination target with strict SSRF filtering.
/// Fails closed if the destination IP is in private, loopback, or metadata ranges.
pub fn try_connect_target_socket(host_port: &str) -> Option<TcpStream> {
    let addrs: Vec<_> = host_port.to_socket_addrs().ok()?.collect();
    for addr in &addrs {
        if is_private_or_restricted(&addr.ip()) {
            eprintln!(
                "[proxy-guard] SSRF blocked direct connection to restricted IP: {}",
                addr.ip()
            );
            continue;
        }
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

/// Performs RFC 1928 / RFC 1929 SOCKS5 client handshake and establishes CONNECT tunnel.
pub fn perform_socks5_connect(
    mut stream: TcpStream,
    target_host: &str,
    target_port: u16,
    auth: Option<&ProxyAuth>,
) -> Option<TcpStream> {
    // 1. Send method negotiation
    if auth.is_some() {
        stream.write_all(&[0x05, 0x02, 0x00, 0x02]).ok()?;
    } else {
        stream.write_all(&[0x05, 0x01, 0x00]).ok()?;
    }

    let mut method_resp = [0u8; 2];
    stream.read_exact(&mut method_resp).ok()?;
    if method_resp[0] != 0x05 {
        return None;
    }

    // 2. Authentication if required
    if method_resp[1] == 0x02 {
        let a = auth?;
        let u_bytes = a.username.as_bytes();
        let p_bytes = a.password.as_bytes();
        if u_bytes.len() > 255 || p_bytes.len() > 255 {
            return None;
        }
        let mut auth_req = Vec::with_capacity(3 + u_bytes.len() + p_bytes.len());
        auth_req.push(0x01);
        auth_req.push(u_bytes.len() as u8);
        auth_req.extend_from_slice(u_bytes);
        auth_req.push(p_bytes.len() as u8);
        auth_req.extend_from_slice(p_bytes);
        stream.write_all(&auth_req).ok()?;

        let mut auth_resp = [0u8; 2];
        stream.read_exact(&mut auth_resp).ok()?;
        if auth_resp[1] != 0x00 {
            return None;
        }
    } else if method_resp[1] != 0x00 {
        return None;
    }

    // 3. Send CONNECT command
    let h_bytes = target_host.as_bytes();
    if h_bytes.len() > 255 {
        return None;
    }
    let mut connect_cmd = Vec::with_capacity(7 + h_bytes.len());
    connect_cmd.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, h_bytes.len() as u8]);
    connect_cmd.extend_from_slice(h_bytes);
    connect_cmd.extend_from_slice(&target_port.to_be_bytes());
    stream.write_all(&connect_cmd).ok()?;

    // 4. Read reply header [VER, REP, RSV, ATYP]
    let mut rep_hdr = [0u8; 4];
    stream.read_exact(&mut rep_hdr).ok()?;
    if rep_hdr[0] != 0x05 || rep_hdr[1] != 0x00 {
        return None;
    }

    // Discard bind address and port
    match rep_hdr[3] {
        0x01 => {
            let mut addr = [0u8; 4 + 2];
            stream.read_exact(&mut addr).ok()?;
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).ok()?;
            let mut addr = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut addr).ok()?;
        }
        0x04 => {
            let mut addr = [0u8; 16 + 2];
            stream.read_exact(&mut addr).ok()?;
        }
        _ => return None,
    }

    Some(stream)
}
