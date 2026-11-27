//! Pure Parsing & Username Adaptation Helpers for Upstream Proxies.
//!
//! Clean Architecture: Core Domain Layer (Submodule of `domain::proxy`).

use super::proxy::{ProxyAuth, ProxyProtocol};

pub(crate) type ParsedProxyComponents = (
    ProxyProtocol,
    String,
    u16,
    Option<ProxyAuth>,
    Option<String>,
);

pub(crate) fn infer_proxy_tier_tag(host: &str, username: Option<&str>) -> Option<&'static str> {
    let h = host.to_ascii_lowercase();
    let u = username.unwrap_or("").to_ascii_lowercase();
    if h.starts_with("dc.") || u.starts_with("user-dc") || u.contains("-dc_") || u.contains("_dc_")
    {
        Some("datacenter")
    } else if h.starts_with("pr.") || u.starts_with("customer-") || u.contains("resi") {
        Some("residential")
    } else {
        None
    }
}

pub(crate) fn is_country_acronym(s: &str) -> bool {
    let t = s.trim();
    t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic())
}

/// Parses a country input such as `"VN"` or `"-cc-vn"`, returning `(normalized_cc, force_cc_username)`.
pub fn parse_country_input(raw: &str) -> Option<(String, bool)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for prefix in &["-cc-", "_cc_"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let cc = rest.trim();
            if is_country_acronym(cc) {
                return Some((cc.to_ascii_uppercase(), true));
            }
        }
    }
    Some((trimmed.to_ascii_uppercase(), false))
}

/// Extracts an optional inline 2-letter country acronym (`#CC`, `|CC`, `CC|`)
/// or non-country `#tag` fragment (e.g. `#oxylabs`) from the host/URL tail.
pub(crate) fn split_inline_country_and_tag(s: &str) -> (&str, Option<String>, Option<String>) {
    if let Some((base, frag)) = s.rsplit_once('#') {
        let after_userinfo = base.rfind('@').map(|idx| &base[idx + 1..]).unwrap_or(base);
        let frag_trimmed = frag.trim();
        if !base.trim().is_empty() && !after_userinfo.is_empty() {
            if is_country_acronym(frag_trimmed) {
                return (base.trim(), Some(frag_trimmed.to_ascii_uppercase()), None);
            }
            let tag = if frag_trimmed.is_empty() {
                None
            } else {
                Some(frag_trimmed.to_string())
            };
            return (base.trim(), None, tag);
        }
    }
    if let Some((base, suffix)) = s.rsplit_once('|') {
        if is_country_acronym(suffix) && !base.trim().is_empty() {
            return (base.trim(), Some(suffix.trim().to_ascii_uppercase()), None);
        }
    }
    if let Some((prefix, rest)) = s.split_once('|') {
        if is_country_acronym(prefix) && !rest.trim().is_empty() {
            return (rest.trim(), Some(prefix.trim().to_ascii_uppercase()), None);
        }
    }
    (s, None, None)
}

/// Helper to parse URL or vendor string `host:port:user:pass[:CC]`
pub(crate) fn parse_proxy_components(s: &str) -> Result<ParsedProxyComponents, String> {
    if s.contains("://") {
        let parts: Vec<&str> = s.splitn(2, "://").collect();
        let protocol = ProxyProtocol::parse(parts[0]);
        let rest = parts[1];

        let (auth, host_port) = if let Some(idx) = rest.find('@') {
            let auth_part = &rest[..idx];
            let hp = &rest[idx + 1..];
            let creds: Vec<&str> = auth_part.splitn(2, ':').collect();
            let auth = if creds.len() == 2 {
                Some(ProxyAuth {
                    username: url_decode(creds[0]),
                    password: url_decode(creds[1]),
                })
            } else {
                None
            };
            (auth, hp)
        } else {
            (None, rest)
        };

        let clean_hp = host_port.split(['/', '?', '#']).next().unwrap_or(host_port);
        let (host, port) = parse_host_port(clean_hp)?;

        Ok((protocol, host, port, auth, None))
    } else if s.starts_with('[') {
        let (host, remainder) = if let Some(close_idx) = s.find(']') {
            (s[..=close_idx].to_string(), &s[close_idx + 1..])
        } else {
            return Err("Mismatched brackets in IPv6 address".to_string());
        };
        let rest = remainder.strip_prefix(':').unwrap_or(remainder);
        let segments: Vec<&str> = rest.split(':').collect();
        match segments.len() {
            1 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None, None))
            }
            3 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[1].to_string(),
                    password: segments[2].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth, None))
            }
            4 if is_country_acronym(segments[3]) => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[1].to_string(),
                    password: segments[2].to_string(),
                });
                Ok((
                    ProxyProtocol::Http,
                    host,
                    port,
                    auth,
                    Some(segments[3].to_ascii_uppercase()),
                ))
            }
            _ => Err(
                "Invalid IPv6 proxy format. Expected '[ipv6]:port' or '[ipv6]:port:user:pass[:CC]'"
                    .to_string(),
            ),
        }
    } else {
        let segments: Vec<&str> = s.split(':').collect();
        match segments.len() {
            2 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None, None))
            }
            4 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[2].to_string(),
                    password: segments[3].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth, None))
            }
            5 if is_country_acronym(segments[4]) => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[2].to_string(),
                    password: segments[3].to_string(),
                });
                Ok((
                    ProxyProtocol::Http,
                    host,
                    port,
                    auth,
                    Some(segments[4].to_ascii_uppercase()),
                ))
            }
            _ => Err(
                "Invalid proxy format. Expected 'protocol://host:port' or 'host:port:user:pass[:CC]'"
                    .to_string(),
            ),
        }
    }
}

fn parse_host_port(hp: &str) -> Result<(String, u16), String> {
    if hp.starts_with('[') {
        if let Some(close_idx) = hp.find(']') {
            let host = hp[..=close_idx].to_string();
            let remainder = &hp[close_idx + 1..];
            let port = if let Some(stripped) = remainder.strip_prefix(':') {
                stripped
                    .parse::<u16>()
                    .map_err(|_| "Invalid proxy port".to_string())?
            } else {
                8080
            };
            Ok((host, port))
        } else {
            Err("Mismatched brackets in IPv6 address".to_string())
        }
    } else {
        let hp_parts: Vec<&str> = hp.split(':').collect();
        if hp_parts.is_empty() || hp_parts[0].is_empty() {
            return Err("Invalid proxy host".to_string());
        }
        let host = hp_parts[0].to_string();
        let port: u16 = if hp_parts.len() > 1 {
            hp_parts[1]
                .parse()
                .map_err(|_| "Invalid proxy port".to_string())?
        } else {
            8080
        };
        Ok((host, port))
    }
}

pub(crate) fn url_decode(s: &str) -> String {
    let mut decoded = Vec::with_capacity(s.len());
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            if let (Some(&h1), Some(&h2)) = (bytes.next(), bytes.next()) {
                let hex_str = [h1, h2];
                if let Ok(hex) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        decoded.push(byte);
                        continue;
                    }
                }
                decoded.push(b'%');
                decoded.push(h1);
                decoded.push(h2);
            } else {
                decoded.push(b'%');
            }
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| s.to_string())
}

pub(crate) fn url_encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";
                out.push(HEX_CHARS[(b >> 4) as usize] as char);
                out.push(HEX_CHARS[(b & 0x0F) as usize] as char);
            }
        }
    }
    out
}

pub(crate) fn extract_country_from_username(username: &str) -> Option<String> {
    let u = username.to_ascii_lowercase();
    for pattern in &["-cc-", "_cc_", "-country-", "_country-", "-zone-"] {
        if let Some(pos) = u.find(pattern) {
            let after = &u[pos + pattern.len()..];
            let code: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if code.len() == 2 {
                return Some(code.to_ascii_uppercase());
            }
        }
    }
    None
}

pub(crate) fn replace_country_in_username(username: &str, target_country: &str) -> Option<String> {
    let u_lower = username.to_ascii_lowercase();
    let target = target_country.trim().to_ascii_lowercase();
    if target.len() != 2 || !target.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    for pattern in &["-cc-", "_cc_", "-country-", "_country-", "-zone-"] {
        if let Some(pos) = u_lower.find(pattern) {
            let prefix = &username[..pos + pattern.len()];
            let remainder = &username[pos + pattern.len()..];
            let code_len = remainder
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .count();
            if code_len == 2 {
                let suffix = &remainder[code_len..];
                let formatted = if *pattern == "-country-" || *pattern == "_country-" {
                    target.to_ascii_uppercase()
                } else {
                    target.clone()
                };
                return Some(format!("{prefix}{formatted}{suffix}"));
            }
        }
    }
    if (u_lower.starts_with("user-dc") || u_lower.contains("-dc_") || u_lower.contains("_dc_"))
        && !u_lower.contains("-country-")
    {
        let target_upper = target.to_ascii_uppercase();
        return Some(format!("{username}-country-{target_upper}"));
    }
    if (u_lower.starts_with("customer-") || u_lower.contains("resi")) && !u_lower.contains("-cc-") {
        return Some(format!("{username}-cc-{target}"));
    }
    None
}
