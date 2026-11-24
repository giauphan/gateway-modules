//! Integration tests for the request optimizer and expanded rule set.

use proxy_guard::request_optimizer::{
    count_stripped_params, optimize_request_headers, strip_tracking_params_from_url,
};
use proxy_guard::rules_engine::RulesEngine;

// ── Tracking param stripping ──────────────────────────────────────────────────

#[test]
fn test_strip_all_google_tracking_params() {
    let url = "/page?id=42&utm_source=google&utm_medium=cpc&utm_campaign=buy&gclid=abc123";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/page?id=42");
    assert!(!result.contains("utm_"));
    assert!(!result.contains("gclid"));
}

#[test]
fn test_strip_facebook_params() {
    let url = "/checkout?product=99&fbclid=IwAR3xyz&color=blue";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/checkout?product=99&color=blue");
}

#[test]
fn test_strip_hubspot_params() {
    let url = "/lp?_hsenc=enc123&_hsmi=456&legit_param=value";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/lp?legit_param=value");
}

#[test]
fn test_all_params_stripped_removes_question_mark() {
    let url = "/beacon?utm_source=email&fbclid=x&gclid=y&ttclid=z";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/beacon");
}

#[test]
fn test_no_query_params_unchanged() {
    let url = "/api/v1/products";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/api/v1/products");
}

#[test]
fn test_empty_query_string_cleaned() {
    let url = "/page?";
    let result = strip_tracking_params_from_url(url);
    assert_eq!(result, "/page");
}

#[test]
fn test_count_stripped_params_multiple() {
    let url = "/p?id=1&utm_source=x&utm_campaign=y&fbclid=z&gclid=w&price=99";
    assert_eq!(count_stripped_params(url), 4);
}

#[test]
fn test_count_stripped_params_none() {
    let url = "/p?id=1&category=shoes";
    assert_eq!(count_stripped_params(url), 0);
}

// ── HTTP header optimization ──────────────────────────────────────────────────

#[test]
fn test_optimize_strips_fingerprinting_headers() {
    let raw = b"GET /page HTTP/1.1\r\nHost: example.com\r\nsec-ch-ua: \"Chromium\";v=\"120\"\r\nsec-ch-ua-mobile: ?0\r\nsec-ch-ua-platform: \"Linux\"\r\nReferer: https://google.com\r\n\r\n";
    let optimized = optimize_request_headers(raw);
    let s = std::str::from_utf8(&optimized).unwrap();
    assert!(
        !s.to_ascii_lowercase().contains("sec-ch-ua"),
        "sec-ch-ua should be stripped"
    );
    assert!(
        !s.to_ascii_lowercase().contains("referer"),
        "Referer should be stripped"
    );
    assert!(s.contains("example.com"), "Host should be preserved");
}

#[test]
fn test_optimize_forces_gzip_encoding() {
    let raw = b"GET / HTTP/1.1\r\nHost: shop.com\r\n\r\n";
    let optimized = optimize_request_headers(raw);
    let s = std::str::from_utf8(&optimized).unwrap();
    assert!(
        s.contains("Accept-Encoding: gzip, deflate"),
        "gzip should be injected: {s}"
    );
}

#[test]
fn test_optimize_replaces_brotli_with_gzip() {
    let raw =
        b"GET / HTTP/1.1\r\nHost: shop.com\r\nAccept-Encoding: br, zstd, gzip, deflate\r\n\r\n";
    let optimized = optimize_request_headers(raw);
    let s = std::str::from_utf8(&optimized).unwrap();
    // Should normalize to gzip only
    assert!(s.contains("Accept-Encoding: gzip, deflate"));
    // Should not have duplicate Accept-Encoding
    assert_eq!(s.matches("Accept-Encoding").count(), 1);
    // brotli (br) should be dropped since brotli requires extra CPU to decompress
    // but most importantly Content-Length/chunking mismatches are avoided
}

#[test]
fn test_optimize_forces_connection_close() {
    let raw = b"GET / HTTP/1.1\r\nHost: shop.com\r\nConnection: keep-alive\r\n\r\n";
    let optimized = optimize_request_headers(raw);
    let s = std::str::from_utf8(&optimized).unwrap();
    assert!(
        s.contains("Connection: close"),
        "Should force connection close"
    );
    assert_eq!(
        s.matches("Connection:").count(),
        1,
        "Should not duplicate Connection header"
    );
}

#[test]
fn test_optimize_strips_x_forwarded_for() {
    let raw =
        b"POST /api HTTP/1.1\r\nHost: api.example.com\r\nX-Forwarded-For: 1.2.3.4\r\nContent-Type: application/json\r\n\r\n";
    let optimized = optimize_request_headers(raw);
    let s = std::str::from_utf8(&optimized).unwrap();
    assert!(
        !s.to_ascii_lowercase().contains("x-forwarded-for"),
        "x-forwarded-for should be stripped"
    );
    assert!(
        s.contains("Content-Type: application/json"),
        "Content-Type should be preserved"
    );
}

// ── Expanded rule catalog ─────────────────────────────────────────────────────

#[test]
fn test_new_rules_block_pinterest_tiktok_snapchat() {
    let engine = RulesEngine::with_builtin_rules();

    // Pinterest
    let d = engine.evaluate("https://pintrk.com/track", "GET", None);
    assert!(d.is_blocked(), "Pinterest tracker should be blocked");

    // TikTok monitor
    let d = engine.evaluate("https://mon.tiktok.com/v3/user/", "POST", None);
    assert!(d.is_blocked(), "TikTok monitor should be blocked");

    // Snapchat pixel
    let d = engine.evaluate("https://sc-static.net/scevent.min.js", "GET", None);
    assert!(d.is_blocked(), "Snapchat pixel should be blocked");
}

#[test]
fn test_new_rules_block_mixpanel_amplitude_posthog() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate("https://api.mixpanel.com/track/", "POST", None);
    assert!(d.is_blocked(), "Mixpanel should be blocked");

    let d = engine.evaluate("https://api2.amplitude.com/2/httpapi", "POST", None);
    assert!(d.is_blocked(), "Amplitude should be blocked");

    let d = engine.evaluate("https://app.posthog.com/capture/", "POST", None);
    assert!(d.is_blocked(), "PostHog should be blocked");
}

#[test]
fn test_new_rules_block_google_fonts() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate(
        "https://fonts.googleapis.com/css2?family=Inter:wght@400;700",
        "GET",
        None,
    );
    assert!(d.is_blocked(), "Google Fonts CSS should be blocked");

    let d = engine.evaluate(
        "https://fonts.gstatic.com/s/inter/v13/UcCO.woff2",
        "GET",
        None,
    );
    assert!(d.is_blocked(), "Google Fonts binary should be blocked");
}

#[test]
fn test_new_rules_block_sourcemaps_and_favicons() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate("https://cdn.example.com/app.js.map", "GET", None);
    assert!(d.is_blocked(), ".js.map source map should be blocked");

    let d = engine.evaluate("https://example.com/favicon.ico", "GET", None);
    assert!(d.is_blocked(), "favicon.ico should be blocked");

    let d = engine.evaluate("https://app.example.com/manifest.json", "GET", None);
    assert!(d.is_blocked(), "manifest.json should be blocked");
}

#[test]
fn test_new_rules_block_hls_video() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate("https://cdn.example.com/video/playlist.m3u8", "GET", None);
    assert!(d.is_blocked(), "HLS playlist .m3u8 should be blocked");

    let d = engine.evaluate("https://cdn.example.com/segment/0001.ts", "GET", None);
    assert!(d.is_blocked(), "HLS .ts segment should be blocked");
}

#[test]
fn test_new_rules_block_newrelic_intercom() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate("https://bam.nr-data.net/events/1/", "POST", None);
    assert!(d.is_blocked(), "New Relic BAM should be blocked");

    let d = engine.evaluate("https://api.intercom.io/messenger/web/ping", "POST", None);
    assert!(d.is_blocked(), "Intercom should be blocked");
}

#[test]
fn test_new_rules_block_adroll_pubmatic() {
    let engine = RulesEngine::with_builtin_rules();

    let d = engine.evaluate("https://d.adroll.com/pixel/retarget", "GET", None);
    assert!(d.is_blocked(), "AdRoll should be blocked");

    let d = engine.evaluate("https://ib.pubmatic.com/AdServer/", "GET", None);
    assert!(d.is_blocked(), "PubMatic should be blocked");
}

#[test]
fn test_ga4_g_collect_blocked() {
    let engine = RulesEngine::with_builtin_rules();
    let d = engine.evaluate(
        "https://region1.google-analytics.com/g/collect?v=2&tid=G-XXXX",
        "POST",
        None,
    );
    assert!(d.is_blocked(), "GA4 g/collect endpoint should be blocked");
}
