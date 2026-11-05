//! Integration regression tests for v1.17.0 audit hardening:
//! 1. False-positive VIDEO_READY from self-declared audit provenance, malformed visual media and unchecked license evidence.
//! 2. Incomplete source revision invalidation caused by hashing only the first 500 body characters.
//! 3. Stale cache and consumed-candidate reappearance across discovery paths.
//! 4. Feed-observed timestamps incorrectly treated as publication times.
//! 5. Config updates not rebuilding live feed providers, plus ineffective default_sources/max_age_days.
//! 6. Incorrect timezone/date parsing.
//! 7. HTTP method guards, request limits and current MCP compatibility.

use daily_news::date::parse_iso_to_epoch;
use daily_news::domain::{
    ClaimEvidence, EvidenceRef, RawNewsItem, ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::handlers::handle_request;
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    evaluate_readiness_full, is_valid_visual_asset_format,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::url::{compute_content_hash, compute_sha256, compute_sha256_bytes};
use serde_json::json;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_v117_{tag}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct MockTestFetcher;

impl daily_news::ports::FeedFetcherPort for MockTestFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(vec![])
    }
    fn fetch_sources(
        &self,
        _sources: &[String],
        _limit: usize,
    ) -> Result<Vec<RawNewsItem>, String> {
        Ok(vec![])
    }
}

fn build_test_service(base_dir: &std::path::Path) -> (DailyNewsService, Arc<FileStorageAdapter>) {
    let storage = Arc::new(FileStorageAdapter::new(base_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(base_dir));
    let fetcher = Arc::new(MockTestFetcher);
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer).with_source_capture(capturer);
    (service, storage)
}

// ---------------------------------------------------------------------------
// Issue 1: False-positive VIDEO_READY
// ---------------------------------------------------------------------------
#[test]
fn test_issue1_malformed_visual_media_fails_validation() {
    // A loose 4-byte string "PNG " or "PNG!" must NOT pass as a valid PNG
    let fake_png = b"PNG! corrupt data without proper PNG magic bytes";
    assert!(
        !is_valid_visual_asset_format(fake_png),
        "Malformed PNG missing 8-byte PNG signature must fail format validation"
    );

    // PNG with truncated IHDR (< 33 bytes or without IHDR chunk) must fail
    let truncated_png = b"\x89PNG\r\n\x1a\n\x00\x00\x00";
    assert!(
        !is_valid_visual_asset_format(truncated_png),
        "Truncated PNG header must fail format validation"
    );

    // GIF with truncated screen descriptor (< 13 bytes) must fail
    let truncated_gif = b"GIF89a";
    assert!(
        !is_valid_visual_asset_format(truncated_gif),
        "GIF missing logical screen descriptor must fail format validation"
    );

    // SVG containing injection script tags or lacking closing </svg> must fail
    let malicious_svg = b"<svg><script>alert('xss')</script></svg>";
    assert!(
        !is_valid_visual_asset_format(malicious_svg),
        "SVG with script tags must fail validation"
    );
    let unclosed_svg = b"<svg width='100' height='100'>No closing tag";
    assert!(
        !is_valid_visual_asset_format(unclosed_svg),
        "SVG missing closing tag must fail validation"
    );
}

#[test]
fn test_issue1_self_declared_audit_and_unchecked_license_evidence_fails_video_ready() {
    let source_text = "Detailed whitepaper with reproducible empirical benchmarks achieving 4x latency reduction.";
    let source_sha = compute_sha256(source_text);
    let primary_source = daily_news::domain::PrimarySourceEvidence {
        url: "https://example.com/paper".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("System Benchmark Whitepaper".to_string()),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        excerpt: Some(source_text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{source_sha}.txt")),
        snapshot_sha256: Some(source_sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    // Audit document contains self-assessment / self-audit disclaimer
    let self_audit_text = "Audit Report: The internal engineering team performed a self-assessment and self-evaluation benchmark. CertiK tools were used to verify 4x latency reduction pass findings.";
    let self_audit_sha = compute_sha256(self_audit_text);

    let claims = vec![ClaimEvidence {
        claim_id: "claim_audit_1".to_string(),
        statement: "4x latency reduction".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha.clone(),
            excerpt: "empirical benchmarks achieving 4x latency reduction".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_self_audit".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "internal_lead".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(self_audit_sha.clone()),
            secondary_source_ref: None,
        }),
    }];

    // Valid PNG asset with positive dimensions
    let mut valid_png = Vec::new();
    valid_png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    valid_png.extend_from_slice(&13u32.to_be_bytes());
    valid_png.extend_from_slice(b"IHDR");
    valid_png.extend_from_slice(&640u32.to_be_bytes());
    valid_png.extend_from_slice(&480u32.to_be_bytes());
    valid_png.extend_from_slice(&[8, 2, 0, 0, 0]);
    valid_png.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
    let valid_png_sha = compute_sha256_bytes(&valid_png);

    // Unchecked license evidence: snapshot hash that does not exist or license doc that is empty
    let unresolvable_license_hash = "f".repeat(64);
    let visuals = vec![VisualEvidence {
        asset_ref: "vis_chart_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(valid_png_sha.clone()),
        license_evidence_ref: Some(unresolvable_license_hash),
        verified_rights: Some(true),
    }];

    let audit_resolver = move |h: &str| -> Option<String> {
        if h == self_audit_sha {
            Some(self_audit_text.to_string())
        } else {
            None
        }
    };
    let asset_resolver = move |h: &str| -> Option<Vec<u8>> {
        if h == valid_png_sha {
            Some(valid_png.clone())
        } else {
            None
        }
    };
    let secondary_resolver = |_h: &str| -> Option<String> { None };

    let readiness = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &claims,
        &visuals,
        daily_news::domain::ContentOrigin::Article,
        Some(&secondary_resolver),
        Some(&audit_resolver),
        Some(&asset_resolver),
    );

    // Must NOT be VideoReady due to self-audit and unverified license evidence
    assert_ne!(
        readiness.status,
        ReadinessStatus::VideoReady,
        "Self-declared audit provenance and unchecked license evidence must not yield VideoReady"
    );
}

// ---------------------------------------------------------------------------
// Issue 2: Incomplete source revision invalidation (hashing past 500 chars)
// ---------------------------------------------------------------------------
#[test]
fn test_issue2_content_hash_must_reflect_body_changes_beyond_500_chars() {
    let title = "High-Performance Rust Network Engine";
    let url = "https://example.com/engine";

    // 600 characters of base body text
    let base_prefix = "A".repeat(520);
    let body_v1 = format!("{base_prefix} Initial Benchmark: 100k req/sec.");
    let body_v2 = format!("{base_prefix} Updated Benchmark: 500k req/sec.");

    let hash_v1 = compute_content_hash(title, Some(&body_v1), url);
    let hash_v2 = compute_content_hash(title, Some(&body_v2), url);

    assert_ne!(
        hash_v1, hash_v2,
        "Modifications occurring beyond 500 characters must yield distinct content_hash values"
    );
}

// ---------------------------------------------------------------------------
// Issue 3: Stale cache and consumed-candidate reappearance
// ---------------------------------------------------------------------------
#[test]
fn test_issue3_consumed_candidate_must_not_reappear_in_candidates_or_cache() {
    let tmp = make_temp_dir("consumed_reappear");
    let (service, storage) = build_test_service(&tmp);

    // 1. Ingest a candidate
    let mut raw = RawNewsItem::new(
        "raw_c1",
        "https://example.com/article_c1",
        "Deterministic Storage Release",
        "2026-11-04T06:00:00Z",
    );
    raw.publisher = Some("TechPost".to_string());
    raw.body = Some("Deterministic storage engine released.".to_string());
    let cand = service.normalize_item(&raw).expect("normalize");
    let cid = cand.candidate_id.clone();

    // Save into storage
    storage
        .save_candidates(&[cand])
        .expect("save");

    // 2. Candidate is returned initially
    let list1 = service.get_candidates(10);
    assert!(
        list1.iter().any(|c| c.candidate_id == cid),
        "Unconsumed candidate must appear in get_candidates"
    );

    // 3. Mark candidate as consumed
    service.consume(std::slice::from_ref(&cid)).expect("consume");
    assert!(storage.is_consumed(&cid));

    // 4. In get_candidates, consumed candidate MUST NOT appear
    let list2 = service.get_candidates(10);
    assert!(
        !list2.iter().any(|c| c.candidate_id == cid),
        "Consumed candidate must never reappear in get_candidates"
    );

    // 5. In chat_completion, consumed candidate MUST NOT appear
    let chat = service.chat_completion(10);
    assert!(
        !chat.contains(&cid),
        "Consumed candidate must not appear in chat completion"
    );

    // 6. In discovery try_cache_tier, consumed candidate MUST NOT appear
    let disc_res = service
        .discover(10, None, Some(vec![]), None, None, None)
        .expect("discover");
    assert!(
        !disc_res.candidates.iter().any(|c| c.candidate_id == cid),
        "Consumed candidate must never appear in discovery results"
    );
}

// ---------------------------------------------------------------------------
// Issue 4: Feed-observed timestamps vs publication times
// ---------------------------------------------------------------------------
#[test]
fn test_issue4_reddit_external_link_is_feed_observed_not_article_publication() {
    let d = json!({
        "id": "post_123",
        "url": "https://techcrunch.com/2026/11/04/new-ai-model",
        "title": "Startup releases new AI model",
        "created_utc": 1793780000,
        "author": "tech_poster",
        "score": 42,
        "num_comments": 15,
        "selftext": "",
        "subreddit": "MachineLearning"
    });

    let item = daily_news::reddit::parse_reddit_post_data(&d, "MachineLearning", None)
        .expect("Parsed reddit post");

    // For external link, submission time is feed-observed
    assert_eq!(
        item.published_at_kind,
        daily_news::domain::PublishedAtKind::FeedObserved,
        "External reddit post submission time must be classified as FeedObserved"
    );
}

// ---------------------------------------------------------------------------
// Issue 5: Config updates rebuild live feed providers and support default_sources/max_age_days
// ---------------------------------------------------------------------------
#[test]
fn test_issue5_config_updates_default_sources_and_max_age_days() {
    let tmp = make_temp_dir("config_rebuild");
    let (service, _storage) = build_test_service(&tmp);

    let update_payload = json!({
        "default_sources": ["hn", "reddit"],
        "max_age_days": 14,
        "redis_enabled": true,
        "redis_url": "redis://127.0.0.1:6379/1"
    });

    let resp = handle_request(
        json!({
            "path": "/config",
            "method": "POST",
            "body": update_payload
        }),
        &service,
        &tmp,
    );

    assert_eq!(resp["status"], 200);

    let cfg = service.get_config();
    assert_eq!(cfg.default_sources, vec!["hn".to_string(), "reddit".to_string()]);
    assert_eq!(cfg.max_age_days, 14);
    assert!(cfg.redis_enabled);
}

// ---------------------------------------------------------------------------
// Issue 6: Date and timezone parsing
// ---------------------------------------------------------------------------
#[test]
fn test_issue6_timezone_offsets_parsed_correctly_to_utc() {
    // 2026-11-04T15:30:00+07:00 in UTC is 2026-11-04T08:30:00Z
    let ts_pos = "2026-11-04T15:30:00+07:00";
    let ts_utc = "2026-11-04T08:30:00Z";
    let epoch_pos = parse_iso_to_epoch(ts_pos).expect("parse positive offset");
    let epoch_utc = parse_iso_to_epoch(ts_utc).expect("parse UTC");
    assert_eq!(
        epoch_pos, epoch_utc,
        "Positive timezone offset +07:00 must correctly normalize to UTC epoch"
    );

    // 2026-11-04T03:30:00-05:00 in UTC is 2026-11-04T08:30:00Z
    let ts_neg = "2026-11-04T03:30:00-05:00";
    let epoch_neg = parse_iso_to_epoch(ts_neg).expect("parse negative offset");
    assert_eq!(
        epoch_neg, epoch_utc,
        "Negative timezone offset -05:00 must correctly normalize to UTC epoch"
    );
}

// ---------------------------------------------------------------------------
// Issue 7: HTTP method guards, request limits, and MCP compatibility
// ---------------------------------------------------------------------------
#[test]
fn test_issue7_http_method_guards_and_request_limits() {
    let tmp = make_temp_dir("method_guards");
    let (service, _storage) = build_test_service(&tmp);

    // 1. DELETE on /health must return 405 Method Not Allowed
    let resp_health_del = handle_request(
        json!({
            "path": "/health",
            "method": "DELETE"
        }),
        &service,
        &tmp,
    );
    assert_eq!(
        resp_health_del["status"], 405,
        "DELETE on /health must return 405 Method Not Allowed"
    );

    // 2. GET on /consume must return 405 Method Not Allowed (consume requires POST)
    let resp_consume_get = handle_request(
        json!({
            "path": "/consume",
            "method": "GET"
        }),
        &service,
        &tmp,
    );
    assert_eq!(
        resp_consume_get["status"], 405,
        "GET on /consume must return 405 Method Not Allowed"
    );

    // 3. Payload larger than 2MB must return 413 Payload Too Large
    let oversized_body = "x".repeat(3 * 1024 * 1024);
    let resp_oversized = handle_request(
        json!({
            "path": "/discover",
            "method": "POST",
            "body": oversized_body
        }),
        &service,
        &tmp,
    );
    assert_eq!(
        resp_oversized["status"], 413,
        "Oversized request body must return 413 Payload Too Large"
    );

    // 4. MCP JSON-RPC initialize protocol negotiation
    let mcp_init = handle_request(
        json!({
            "path": "/mcp",
            "method": "POST",
            "body": {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "clientInfo": { "name": "test-agent", "version": "1.0.0" }
                }
            }
        }),
        &service,
        &tmp,
    );
    assert_eq!(mcp_init["status"], 200);
}
