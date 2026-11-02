use daily_news::*;
use std::collections::HashSet;

#[test]
fn test_canonicalize_url() {
    let url =
        "https://example.com/posts/1/?utm_source=twitter&utm_medium=social&fbclid=xyz#section";
    assert_eq!(canonicalize_url(url), "https://example.com/posts/1");
    assert_eq!(
        canonicalize_url("http://example.com/path/"),
        "http://example.com/path"
    );
}

#[test]
fn test_normalize_publisher() {
    assert_eq!(normalize_publisher(Some("The Verge.com")), "The Verge");
    assert_eq!(normalize_publisher(Some("  TechCrunch.io  ")), "TechCrunch");
    assert_eq!(normalize_publisher(None), "Unknown");
    assert_eq!(normalize_publisher(Some("")), "Unknown");
}

#[test]
fn test_parse_iso_and_boundary() {
    assert!(parse_boundary_datetime("", false).is_err());
    assert!(parse_boundary_datetime("   ", false).is_err());

    let dt_start = parse_boundary_datetime("2026-10-28", false).unwrap();
    let dt_end = parse_boundary_datetime("2026-10-28", true).unwrap();
    assert_eq!(dt_end - dt_start, 86399);

    let dt_iso = parse_boundary_datetime("2026-11-01T15:30:00Z", false).unwrap();
    assert!(dt_iso > dt_start);

    assert!(parse_boundary_datetime("not-a-valid-date", false).is_err());
}

#[test]
fn test_compute_hashes() {
    let t1 = "OpenAI Announces New Agent SDK";
    let t2 = "openai  announces new agent sdk ";
    assert_eq!(compute_title_hash(t1), compute_title_hash(t2));

    let c1 = compute_content_hash(t1, Some("Body text here"), "https://example.com/sdk");
    let c2 = compute_content_hash(t2, Some("Body text here"), "https://example.com/sdk");
    assert_eq!(c1, c2);
}

#[test]
fn test_is_safe_url() {
    let mut allowed = HashSet::new();
    allowed.insert("github.com".to_string());
    allowed.insert("anthropic.com".to_string());

    assert!(is_safe_url(
        "https://github.com/torvalds/linux",
        Some(&allowed)
    ));
    assert!(is_safe_url(
        "https://docs.anthropic.com/en/docs",
        Some(&allowed)
    ));
    assert!(!is_safe_url(
        "https://malicious-site.com/hack",
        Some(&allowed)
    ));
    assert!(!is_safe_url("http://192.168.1.1/secret", Some(&allowed)));
    assert!(!is_safe_url("http://10.0.0.1/private", Some(&allowed)));
    assert!(is_safe_url("http://127.0.0.1:6379/status", None));
}
