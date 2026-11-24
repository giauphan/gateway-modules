//! HTTP Request Optimizer for Outgoing Upstream Requests.
//!
//! Clean Architecture: Infrastructure Adapter Layer.
//! Strips unnecessary headers and query parameters before forwarding to paid upstream proxies,
//! directly reducing bytes-on-wire and therefore proxy bandwidth costs.

pub mod constants;
pub use constants::*;

/// Rewrites an HTTP header block to minimize bytes-on-wire to the upstream proxy.
///
/// Performs three operations:
/// 1. Strips unnecessary tracking/fingerprinting request headers.
/// 2. Strips tracking query parameters from the request URL.
/// 3. Forces `Accept-Encoding: gzip, deflate` to ensure compressed responses.
///
/// Returns the rewritten header bytes. If parsing fails, returns the original unchanged.
pub fn optimize_request_headers(raw_headers: &[u8]) -> Vec<u8> {
    let header_str = match std::str::from_utf8(raw_headers) {
        Ok(s) => s,
        Err(_) => return raw_headers.to_vec(),
    };

    let mut lines = header_str.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => return raw_headers.to_vec(),
    };

    // Rewrite the request line to strip tracking query params
    let rewritten_request_line = strip_tracking_params_from_request_line(request_line);

    let mut output = Vec::with_capacity(raw_headers.len());
    output.extend_from_slice(rewritten_request_line.as_bytes());
    output.extend_from_slice(b"\r\n");

    // Pass 1: Collect connection options named by original Connection headers
    let mut connection_options = Vec::new();
    let mut header_lines = Vec::new();

    for line in lines {
        if line.is_empty() || line == "\r" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("connection:") {
            for token in val.split(',') {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    connection_options.push(trimmed.to_string());
                }
            }
        }
        header_lines.push(line);
    }

    // Pass 2: Emit filtered and optimized headers
    let mut has_accept_encoding = false;

    for line in header_lines {
        let lower_line = line.to_ascii_lowercase();
        let header_name = get_header_name(&lower_line);

        // Strip headers named by client Connection options
        if connection_options.iter().any(|opt| opt == header_name) {
            continue;
        }

        // Connection header will be emitted as "Connection: close" at the end
        if header_name == "connection" {
            continue;
        }

        // Detect and handle Accept-Encoding
        if let Some(val) = lower_line.strip_prefix("accept-encoding:") {
            has_accept_encoding = true;
            if let Some(optimized) = compute_optimized_accept_encoding(val) {
                output.extend_from_slice(optimized.as_bytes());
            } else {
                // Preserve client's explicit exclusion or unhandled coding
                output.extend_from_slice(line.as_bytes());
                output.extend_from_slice(b"\r\n");
            }
            continue;
        }

        // Skip stripped headers
        if should_strip_header(&lower_line) {
            continue;
        }

        output.extend_from_slice(line.as_bytes());
        output.extend_from_slice(b"\r\n");
    }

    // Add missing headers
    if !has_accept_encoding {
        output.extend_from_slice(b"Accept-Encoding: gzip, deflate\r\n");
    }
    output.extend_from_slice(b"Connection: close\r\n\r\n");

    output
}

fn get_header_name(lower_line: &str) -> &str {
    lower_line.split(':').next().unwrap_or("").trim()
}

fn should_strip_header(lower_line: &str) -> bool {
    for &header in STRIPPED_REQUEST_HEADERS {
        if let Some(rest) = lower_line.strip_prefix(header) {
            if rest.starts_with(':') || rest.starts_with(' ') {
                return true;
            }
        }
    }
    false
}

struct AcceptCoding {
    name: String,
    q: f32,
}

fn parse_accept_encoding(val: &str) -> Vec<AcceptCoding> {
    let mut codings = Vec::new();
    for part in val.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut subparts = part.split(';');
        let name = subparts.next().unwrap_or("").trim().to_string();
        let mut q = 1.0f32;
        for param in subparts {
            let param = param.trim();
            if let Some(q_str) = param.strip_prefix("q=") {
                if let Ok(parsed_q) = q_str.trim().parse::<f32>() {
                    q = parsed_q;
                }
            }
        }
        codings.push(AcceptCoding { name, q });
    }
    codings
}

fn is_coding_acceptable(codings: &[AcceptCoding], target: &str) -> bool {
    let mut wildcard_q = None;
    for c in codings {
        if c.name == target {
            return c.q > 0.0;
        }
        if c.name == "*" {
            wildcard_q = Some(c.q > 0.0);
        }
    }
    wildcard_q.unwrap_or(false)
}

fn compute_optimized_accept_encoding(val: &str) -> Option<String> {
    let trimmed = val.trim();
    if trimmed.is_empty() {
        return None;
    }
    let codings = parse_accept_encoding(trimmed);
    let gzip_ok = is_coding_acceptable(&codings, "gzip");
    let deflate_ok = is_coding_acceptable(&codings, "deflate");

    match (gzip_ok, deflate_ok) {
        (true, true) => Some("Accept-Encoding: gzip, deflate\r\n".to_string()),
        (true, false) => Some("Accept-Encoding: gzip\r\n".to_string()),
        (false, true) => Some("Accept-Encoding: deflate\r\n".to_string()),
        (false, false) => None,
    }
}

/// Strips known tracking query parameters from the HTTP request line.
///
/// Input:  `GET /page?utm_source=google&id=123&fbclid=abc HTTP/1.1`
/// Output: `GET /page?id=123 HTTP/1.1`
pub fn strip_tracking_params_from_request_line(request_line: &str) -> String {
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return request_line.to_string();
    }

    let method = parts[0];
    let url = parts[1];
    let version = parts[2];

    let cleaned_url = strip_tracking_params_from_url(url);
    format!("{method} {cleaned_url} {version}")
}

/// Strips known tracking query parameters from a URL path.
///
/// Handles both relative paths (`/page?utm=x`) and absolute URLs (`http://host/page?utm=x`).
pub fn strip_tracking_params_from_url(url: &str) -> String {
    let (path_part, query_part) = match url.find('?') {
        Some(idx) => (&url[..idx], Some(&url[idx + 1..])),
        None => (url, None),
    };

    let query = match query_part {
        None => return url.to_string(),
        Some("") => return path_part.to_string(),
        Some(q) => q,
    };

    let kept: Vec<&str> = query
        .split('&')
        .filter(|param| {
            let key = param.split('=').next().unwrap_or(*param);
            let lower_key = key.to_ascii_lowercase();
            !STRIPPED_QUERY_PARAMS
                .iter()
                .any(|&s| s.eq_ignore_ascii_case(&lower_key))
        })
        .collect();

    if kept.is_empty() {
        path_part.to_string()
    } else {
        format!("{}?{}", path_part, kept.join("&"))
    }
}

/// Returns the domain cost savings estimate from stripping tracking parameters.
/// Each stripped parameter saves approximately 20–80 bytes on the wire.
pub fn count_stripped_params(url: &str) -> usize {
    let query = match url.find('?') {
        Some(idx) => &url[idx + 1..],
        None => return 0,
    };

    query
        .split('&')
        .filter(|param| {
            let key = param.split('=').next().unwrap_or(*param);
            let lower_key = key.to_ascii_lowercase();
            STRIPPED_QUERY_PARAMS
                .iter()
                .any(|&s| s.eq_ignore_ascii_case(&lower_key))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_utm_params() {
        let url = "/page?id=123&utm_source=google&utm_medium=cpc&utm_campaign=promo";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(cleaned, "/page?id=123");
        assert!(!cleaned.contains("utm_source"));
        assert!(!cleaned.contains("utm_medium"));
        assert!(!cleaned.contains("utm_campaign"));
    }

    #[test]
    fn test_strip_fbclid() {
        let url = "/checkout?product=42&fbclid=IwAR123abc&gclid=xyz";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(cleaned, "/checkout?product=42");
    }

    #[test]
    fn test_strip_omnisend_parameter() {
        let url = "/product/view?id=99&omnisendContactID=abc123xyz";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(cleaned, "/product/view?id=99");
        assert_eq!(count_stripped_params(url), 1);
    }

    #[test]
    fn test_no_query_passthrough() {
        let url = "/api/products";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(cleaned, "/api/products");
    }

    #[test]
    fn test_all_params_stripped_removes_question_mark() {
        let url = "/track?utm_source=g&fbclid=x";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(cleaned, "/track");
    }

    #[test]
    fn test_optimize_request_headers_strips_sec_ch_ua() {
        let raw = b"GET /page HTTP/1.1\r\nHost: example.com\r\nsec-ch-ua: \"Chromium\";v=\"120\"\r\nReferer: https://google.com\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(!s.contains("sec-ch-ua"), "sec-ch-ua should be stripped");
        assert!(!s.contains("Referer"), "Referer should be stripped");
        assert!(s.contains("Accept-Encoding: gzip"), "gzip should be added");
    }

    #[test]
    fn test_optimize_replaces_accept_encoding_with_gzip() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: br, zstd, gzip, deflate\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: gzip, deflate"));
        // Should not have duplicate accept-encoding
        assert_eq!(s.matches("Accept-Encoding").count(), 1);
    }

    #[test]
    fn test_optimize_accept_encoding_excludes_unadvertised_deflate() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: br, gzip\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: gzip\r\n"));
        assert!(!s.contains("deflate"));
    }

    #[test]
    fn test_optimize_accept_encoding_respects_q0() {
        let raw =
            b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: gzip, deflate;q=0\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: gzip\r\n"));
        assert!(!s.contains("deflate\r\n"));
    }

    #[test]
    fn test_optimize_accept_encoding_preserves_br_only() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: br\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: br\r\n"));
        assert!(!s.contains("gzip"));
    }

    #[test]
    fn test_optimize_strips_headers_named_in_connection_header() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nX-Hop-Token: secret\r\nConnection: X-Hop-Token\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(!s.contains("X-Hop-Token"));
        assert!(s.contains("Connection: close"));
    }

    #[test]
    fn test_optimize_preserves_identity_accept_encoding() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: identity\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: identity"));
        assert!(!s.contains("gzip"));
    }

    #[test]
    fn test_optimize_preserves_explicit_gzip_exclusion() {
        let raw =
            b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: gzip;q=0, identity\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: gzip;q=0, identity"));
    }

    #[test]
    fn test_optimize_strips_proxy_authorization() {
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nProxy-Authorization: Basic dXNlcjpwYXNz\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(!s.to_ascii_lowercase().contains("proxy-authorization"));
    }

    #[test]
    fn test_generic_source_and_ref_params_preserved() {
        let url = "/api/v1/inventory?source=warehouse_north&ref=pos_terminal_1";
        let cleaned = strip_tracking_params_from_url(url);
        assert_eq!(
            cleaned,
            "/api/v1/inventory?source=warehouse_north&ref=pos_terminal_1"
        );
    }

    #[test]
    fn test_count_stripped_params() {
        let url = "/p?id=1&utm_source=x&utm_campaign=y&fbclid=z&gclid=w";
        assert_eq!(count_stripped_params(url), 4);
    }
}
