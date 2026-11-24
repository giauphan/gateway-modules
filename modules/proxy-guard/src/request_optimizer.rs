//! HTTP Request Optimizer for Outgoing Upstream Requests.
//!
//! Clean Architecture: Infrastructure Adapter Layer.
//! Strips unnecessary headers and query parameters before forwarding to paid upstream proxies,
//! directly reducing bytes-on-wire and therefore proxy bandwidth costs.

/// Tracking-only query parameters that should be stripped before forwarding.
/// These contribute zero value to automation flows but can add hundreds of bytes to URLs
/// and cause tracking systems to wake up and phone home.
static STRIPPED_QUERY_PARAMS: &[&str] = &[
    // Google / Universal Analytics
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_source_platform",
    "utm_creative_format",
    "utm_marketing_tactic",
    // Google Ads
    "gclid",
    "gclsrc",
    "gbraid",
    "wbraid",
    "dclid",
    // Meta / Facebook
    "fbclid",
    "fb_action_ids",
    "fb_action_types",
    "fb_source",
    "fb_ref",
    // TikTok
    "ttclid",
    // Microsoft Ads
    "msclkid",
    // HubSpot
    "_hsenc",
    "_hsmi",
    "hsa_acc",
    "hsa_cam",
    "hsa_grp",
    "hsa_ad",
    "hsa_src",
    "hsa_tgt",
    "hsa_kw",
    "hsa_mt",
    "hsa_net",
    "hsa_ver",
    // MailChimp
    "mc_cid",
    "mc_eid",
    // Klaviyo
    "_kx",
    // Drip
    "__s",
    // Vero
    "vero_id",
    "vero_conv",
    // Omnisend
    "omnisendContactID",
    // Iterable
    "_itm",
    // Generic click tracking
    "s_kwcid",
    "ef_id",
    "pcrid",
    "pd_rd_r",
    "pd_rd_w",
    "pd_rd_wg",
    "pf_rd_p",
    "pf_rd_r",
];

/// HTTP request headers that should be dropped before forwarding to upstream.
/// These either identify the browser fingerprint, trigger telemetry pipelines,
/// or are unnecessary for headless automation flows.
static STRIPPED_REQUEST_HEADERS: &[&str] = &[
    // Browser fingerprinting headers
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "sec-ch-ua-arch",
    "sec-ch-ua-bitness",
    "sec-ch-ua-full-version",
    "sec-ch-ua-full-version-list",
    "sec-ch-ua-model",
    "sec-ch-ua-wow64",
    "sec-ch-prefers-color-scheme",
    "sec-ch-prefers-reduced-motion",
    // Downstream proxy authorization (never leak client credentials to upstream)
    "proxy-authorization",
    // No referrer sent - reduces analytics triggering
    "referer",
    // Privacy / tracking headers
    "x-forwarded-for",
    "x-real-ip",
    "x-request-id",
    "traceparent",
    "tracestate",
    "baggage",
    // Browser performance hints that waste bytes
    "save-data",
    "device-memory",
    "downlink",
    "ect",
    "rtt",
    "viewport-width",
    "width",
    "dpr",
    "content-dpr",
    // Service Worker header (triggers SW registration which downloads more scripts)
    "service-worker",
];

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

    let mut has_accept_encoding = false;
    let mut has_connection = false;

    for line in lines {
        // Empty line = end of headers
        if line.is_empty() || line == "\r" {
            break;
        }

        let lower_line = line.to_ascii_lowercase();

        // Detect and handle Accept-Encoding
        if let Some(val) = lower_line.strip_prefix("accept-encoding:") {
            has_accept_encoding = true;
            if client_excludes_gzip(val) {
                // Preserve client's explicit exclusion or identity-only coding
                output.extend_from_slice(line.as_bytes());
                output.extend_from_slice(b"\r\n");
            } else {
                // Normalize accepted compression to standard gzip/deflate
                output.extend_from_slice(b"Accept-Encoding: gzip, deflate\r\n");
            }
            continue;
        }

        // Detect Connection header
        if lower_line.starts_with("connection:") {
            has_connection = true;
            // Force Connection: close to avoid keeping connections alive to paid proxy
            output.extend_from_slice(b"Connection: close\r\n");
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
    if !has_connection {
        output.extend_from_slice(b"Connection: close\r\n");
    }

    output.extend_from_slice(b"\r\n");
    output
}

/// Returns true if the header line should be stripped from outgoing requests.
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

/// Checks if the client explicitly excludes gzip or only accepts identity coding.
fn client_excludes_gzip(val: &str) -> bool {
    let trimmed = val.trim();
    if trimmed.contains("gzip;q=0") || trimmed.contains("gzip; q=0") {
        return true;
    }
    let parts: Vec<&str> = trimmed.split(',').map(|s| s.trim()).collect();
    let has_identity = parts
        .iter()
        .any(|&p| p == "identity" || p.starts_with("identity;"));
    let has_gzip = parts.iter().any(|&p| p == "gzip" || p.starts_with("gzip;"));
    let has_wildcard = parts
        .iter()
        .any(|&p| p == "*" || (p.starts_with("*;") && !p.contains("q=0")));

    has_identity && !has_gzip && !has_wildcard
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
            !STRIPPED_QUERY_PARAMS.iter().any(|&s| s == lower_key)
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
            STRIPPED_QUERY_PARAMS.iter().any(|&s| s == lower_key)
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
        let raw = b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept-Encoding: br, zstd, gzip\r\n\r\n";
        let optimized = optimize_request_headers(raw);
        let s = std::str::from_utf8(&optimized).unwrap();
        assert!(s.contains("Accept-Encoding: gzip, deflate"));
        // Should not have duplicate accept-encoding
        assert_eq!(s.matches("Accept-Encoding").count(), 1);
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
