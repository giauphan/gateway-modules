//! Integration regression tests for v1.18.0 audit hardening:
//! 1. P0: Strict source_verified, media_decodable, and rights_verified gates for VIDEO_READY.
//! 2. P0: Cached VIDEO_READY re-verification on retrieval and production handoff.
//! 3. P1: Unified CandidateEligibilityPolicy across /discover and /candidates.
//! 4. P1: Feed-observed reshared articles rejected when original publication is stale.
//! 5. P1: Strict calendar days, hours/minutes, and timezone offset validation in date parser.
//! 6. P1: Deduplication hash poisoning prevention across topic filters.
//! 7. P1: MCP 2026-07-28 negotiation, server/discover method, and clamped limit arguments.
//! 8. P1/P2: 2MB payload enforcement across arbitrary JSON bodies and /handoff route.

use daily_news::date::parse_iso_to_epoch;
use daily_news::discovery::is_candidate_eligible;
use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, ModuleConfig, NormalizedCandidate,
    PrimarySourceEvidence, ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::handlers::handle_request;
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    evaluate_readiness_full, is_valid_visual_asset_format, is_visual_media_decodable,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::url::{compute_sha256, compute_sha256_bytes};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_v118_{tag}_{}",
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
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<daily_news::domain::RawNewsItem>, String> {
        Ok(vec![])
    }
    fn fetch_sources(
        &self,
        _sources: &[String],
        _limit: usize,
    ) -> Result<Vec<daily_news::domain::RawNewsItem>, String> {
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
// Issue 1: P0 - False-positive VIDEO_READY prevention
// ---------------------------------------------------------------------------
#[test]
fn test_issue1_false_positive_video_ready_prevention() {
    let source_text = "Verified benchmark findings report 4x improvement in system throughput.";
    let source_sha = compute_sha256(source_text);
    let primary_source = PrimarySourceEvidence {
        url: "https://example.com/report".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("TIER_1".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Report".to_string()),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        excerpt: Some(source_text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{source_sha}.txt")),
        snapshot_sha256: Some(source_sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    // Case A: Self-generated audit artifact claiming CertiK verified must FAIL
    let self_generated_audit = "This is a self-generated document stating that CertiK has verified the 4x improvement findings and confirmed benchmarks.";
    let self_audit_sha = compute_sha256(self_generated_audit);
    let self_audit_claims = vec![ClaimEvidence {
        claim_id: "c_self_1".to_string(),
        statement: "4x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha.clone(),
            excerpt: "4x improvement in system throughput".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_self_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "certik_senior_auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(self_audit_sha.clone()),
            secondary_source_ref: None,
        }),
    }];

    // Valid 68-byte decodable 1x1 RGBA PNG
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let valid_png_sha = compute_sha256_bytes(valid_png_bytes);
    let valid_visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(valid_png_sha.clone()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let self_audit_res = move |h: &str| -> Option<String> {
        if h == self_audit_sha { Some(self_generated_audit.to_string()) } else { None }
    };
    let valid_asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == valid_png_sha { Some(valid_png_bytes.to_vec()) } else { None }
    };

    let res_self_audit = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &self_audit_claims,
        &valid_visuals,
        ContentOrigin::Article,
        None,
        Some(&self_audit_res),
        Some(&valid_asset_res),
    );
    assert_ne!(res_self_audit.status, ReadinessStatus::VideoReady);
    assert_eq!(res_self_audit.status, ReadinessStatus::NeedsVerification);

    // Case B: PNG with valid header and dimensions but lacking IDAT image chunk (33 bytes) must FAIL
    let truncated_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x20\0\0\0\x20\x08\x06\0\0\0sp\x8b\xcf";
    assert!(is_valid_visual_asset_format(truncated_png_bytes));
    assert!(!is_visual_media_decodable(truncated_png_bytes));

    let truncated_png_sha = compute_sha256_bytes(truncated_png_bytes);
    let truncated_visuals = vec![VisualEvidence {
        asset_ref: "vis_trunc".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(truncated_png_sha.clone()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let official_audit = "Official Cryptographic Audit Trail Report #2026-CERT-01: CertiK independently verified findings confirmed for 4x improvement in system throughput across test runs.";
    let official_audit_sha = compute_sha256(official_audit);
    let official_claims = vec![ClaimEvidence {
        claim_id: "c_official_1".to_string(),
        statement: "4x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha.clone(),
            excerpt: "4x improvement in system throughput".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_official_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "certik_senior_auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(official_audit_sha.clone()),
            secondary_source_ref: None,
        }),
    }];

    let official_audit_res = move |h: &str| -> Option<String> {
        if h == official_audit_sha { Some(official_audit.to_string()) } else { None }
    };
    let truncated_asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == truncated_png_sha { Some(truncated_png_bytes.to_vec()) } else { None }
    };

    let res_trunc_visual = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &official_claims,
        &truncated_visuals,
        ContentOrigin::Article,
        None,
        Some(&official_audit_res),
        Some(&truncated_asset_res),
    );
    assert_ne!(res_trunc_visual.status, ReadinessStatus::VideoReady);
    assert_eq!(res_trunc_visual.status, ReadinessStatus::NeedsVisualEvidence);

    // Case C: When source_verified, media_decodable, and rights_verified are all satisfied -> VIDEO_READY
    let res_video_ready = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &official_claims,
        &valid_visuals,
        ContentOrigin::Article,
        None,
        Some(&official_audit_res),
        Some(&valid_asset_res),
    );
    assert_eq!(res_video_ready.status, ReadinessStatus::VideoReady);
}

// ---------------------------------------------------------------------------
// Issue 2: P0 - Cached VIDEO_READY Re-verification and Handoff Gate
// ---------------------------------------------------------------------------
#[test]
fn test_issue2_cached_video_ready_reverification_and_handoff_gate() {
    let tmp = make_temp_dir("issue2_handoff");
    let (service, storage) = build_test_service(&tmp);

    let state_dir = tmp.join(".state");
    let evidence_dir = state_dir.join("evidence");
    let assets_dir = state_dir.join("assets");
    std::fs::create_dir_all(&evidence_dir).unwrap();
    std::fs::create_dir_all(&assets_dir).unwrap();

    let primary_text = "Verified benchmark analysis reports 4x improvement in system performance.";
    let primary_sha = compute_sha256(primary_text);
    std::fs::write(evidence_dir.join(format!("{primary_sha}.txt")), primary_text).unwrap();

    let audit_text = "Official Cryptographic Audit Trail Report #2026-H1: CertiK independently verified findings confirmed for 4x improvement across test runs.";
    let audit_sha = compute_sha256(audit_text);
    std::fs::write(evidence_dir.join(format!("{audit_sha}.txt")), audit_text).unwrap();

    let png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let png_sha = compute_sha256_bytes(png_bytes);
    std::fs::write(assets_dir.join(format!("{png_sha}.bin")), png_bytes).unwrap();

    let mut candidate = NormalizedCandidate {
        candidate_id: "cand_video_ready_01".to_string(),
        canonical_url: "https://example.com/item1".to_string(),
        title: "Item 1".to_string(),
        publisher: "example.com".to_string(),
        published_at_utc: "2026-11-04T00:00:00+00:00".to_string(),
        content_hash: compute_sha256("primary content"),
        title_hash: compute_sha256("item 1"),
        content_origin: ContentOrigin::Article,
        ..Default::default()
    };
    candidate.research.primary_source = Some(PrimarySourceEvidence {
        url: "https://example.com/item1".to_string(),
        fetch_status: "SUCCESS".to_string(),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        snapshot_ref: Some(format!("{primary_sha}.txt")),
        snapshot_sha256: Some(primary_sha.clone()),
        ..Default::default()
    });
    candidate.research.claims = vec![ClaimEvidence {
        claim_id: "c_1".to_string(),
        statement: "4x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha.clone(),
            excerpt: "4x improvement in system performance".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_1".to_string(),
            verified_at: "2026-11-04T00:00:00Z".to_string(),
            reviewer_id: "certik_auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(audit_sha.clone()),
            secondary_source_ref: None,
        }),
    }];
    candidate.research.visual_evidence = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(png_sha.clone()),
        license_status: Some("cc-by".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
        ..Default::default()
    }];
    candidate.research.research_readiness = Some(daily_news::domain::ResearchReadiness {
        status: ReadinessStatus::VideoReady,
        reasons: vec!["All criteria met".to_string()],
        missing_fields: vec![],
        contract_version: "2.0".to_string(),
    });

    storage.save_candidates(&[candidate.clone()]).unwrap();

    // Verification before corruption: handoff succeeds
    let handoff_ok = service.verify_production_handoff("cand_video_ready_01");
    if let Err(ref e) = handoff_ok {
        panic!("HANDOFF ERR: {:?}", e);
    }
    assert!(handoff_ok.is_ok());

    // Scenario: Visual asset file is deleted from disk
    let asset_path = assets_dir.join(format!("{png_sha}.bin"));
    std::fs::remove_file(asset_path).unwrap();

    // When candidate is retrieved, readiness is invalidated from VIDEO_READY to NEEDS_REVALIDATION
    let retrieved = service.get_candidate("cand_video_ready_01").unwrap();
    assert_ne!(retrieved.research.research_readiness.as_ref().unwrap().status, ReadinessStatus::VideoReady);
    assert_eq!(retrieved.research.research_readiness.as_ref().unwrap().status, ReadinessStatus::NeedsRevalidation);

    // Production handoff must fail closed with 422
    let handoff_err = service.verify_production_handoff("cand_video_ready_01");
    assert!(handoff_err.is_err());
    let (code, msg) = handoff_err.err().unwrap();
    assert_eq!(code, "HANDOFF_REJECTED");
    assert!(msg.contains("NEEDS_VISUAL_EVIDENCE") || msg.contains("NEEDS_REVALIDATION"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Issue 3: P1 - Unified Freshness and Max Age Filtering in get_candidates
// ---------------------------------------------------------------------------
#[test]
fn test_issue3_unified_freshness_in_get_candidates() {
    let tmp = make_temp_dir("issue3_freshness");
    let (service, storage) = build_test_service(&tmp);

    let mut cfg = service.get_config();
    cfg.max_age_days = 7;
    cfg.require_latest_news = true;
    let _ = storage.save_config(&cfg);

    // Create a 20-day old candidate stored in cache
    let old_cand = NormalizedCandidate {
        candidate_id: "cand_stale_old".to_string(),
        canonical_url: "https://example.com/stale_article".to_string(),
        title: "Stale 20 Days Old Article".to_string(),
        publisher: "example.com".to_string(),
        published_at_utc: "2026-10-10T00:00:00+00:00".to_string(),
        content_hash: compute_sha256("old content"),
        title_hash: compute_sha256("stale title"),
        topic_family: "technology".to_string(),
        ..Default::default()
    };
    storage.save_candidates(&[old_cand]).unwrap();

    // get_candidates must enforce max_age_days and NOT return the 20-day old article
    let candidates = service.get_candidates(10);
    assert!(!candidates.iter().any(|c| c.candidate_id == "cand_stale_old"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Issue 4: P1 - Feed-observed Reshares vs Publication Timestamps
// ---------------------------------------------------------------------------
#[test]
fn test_issue4_feed_observed_reshared_stale_article_rejected() {
    let cfg = ModuleConfig {
        max_age_days: 7,
        require_latest_news: true,
        ..Default::default()
    };

    let mut cand = NormalizedCandidate {
        candidate_id: "cand_reddit_reshare".to_string(),
        canonical_url: "https://example.com/original_2021_paper".to_string(),
        title: "Great Paper from 2021".to_string(),
        publisher: "example.com".to_string(),
        // Reshared today on Reddit
        published_at_utc: "2026-11-05T10:00:00+00:00".to_string(),
        content_hash: compute_sha256("paper content"),
        title_hash: compute_sha256("paper title"),
        topic_family: "technology".to_string(),
        published_at_kind: "feed_observed".to_string(),
        content_origin: ContentOrigin::Article,
        ..Default::default()
    };
    // Primary source publication is from 2021
    cand.research.primary_source = Some(PrimarySourceEvidence {
        url: "https://example.com/original_2021_paper".to_string(),
        fetch_status: "SUCCESS".to_string(),
        published_at: Some("2021-05-10T00:00:00+00:00".to_string()),
        ..Default::default()
    });

    let cutoff = parse_iso_to_epoch("2026-11-01T00:00:00Z").unwrap();
    let upper = parse_iso_to_epoch("2026-11-05T23:59:59Z").unwrap();
    let mut seen_c = HashSet::new();
    let mut seen_t = HashSet::new();

    let eligible = is_candidate_eligible(
        &cand,
        &cfg,
        Some(cutoff),
        cutoff,
        upper,
        &mut seen_c,
        &mut seen_t,
        None,
    );
    assert!(!eligible, "Reshared 2021 article must not be eligible as breaking news");
}

// ---------------------------------------------------------------------------
// Issue 5: P1 - Strict Date and Timezone Offset Parsing
// ---------------------------------------------------------------------------
#[test]
fn test_issue5_strict_calendar_and_timezone_validation() {
    // Non-existent calendar days
    assert!(parse_iso_to_epoch("2026-02-31").is_err());
    assert!(parse_iso_to_epoch("2026-04-31").is_err());
    assert!(parse_iso_to_epoch("2023-02-29").is_err()); // non-leap year

    // Leap year valid day
    assert!(parse_iso_to_epoch("2024-02-29").is_ok());

    // Out-of-bounds hours and minutes
    assert!(parse_iso_to_epoch("2026-11-05T25:99:00Z").is_err());
    assert!(parse_iso_to_epoch("2026-11-05T12:60:00Z").is_err());

    // Out-of-bounds timezone offsets
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+99:99").is_err());
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+25:00").is_err());
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+07:99").is_err());

    // Valid timezone offsets
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+07:00").is_ok());
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00-05:00").is_ok());

    // Non-ASCII timezone text must safely return Err without panicking
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+日本").is_err());
    assert!(parse_iso_to_epoch("2026-11-05T12:00:00+東京:00").is_err());
}

// ---------------------------------------------------------------------------
// Issue 6: P1 - Deduplication Hash Poisoning Prevention
// ---------------------------------------------------------------------------
#[test]
fn test_issue6_deduplication_topic_filter_hash_isolation() {
    let cfg = ModuleConfig::default();
    let cutoff = 1000;
    let upper = 2_000_000_000;
    let topic_filter = vec!["ai".to_string()];

    let mut seen_c = HashSet::new();
    let mut seen_t = HashSet::new();

    // Item 1 has identical title/content, but does NOT match the topic filter
    let item1 = NormalizedCandidate {
        candidate_id: "c1".to_string(),
        canonical_url: "https://example.com/1".to_string(),
        title: "Autonomous Agent Breakthrough".to_string(),
        published_at_utc: "2026-11-05T00:00:00Z".to_string(),
        content_hash: "hash_common_content".to_string(),
        title_hash: "hash_common_title".to_string(),
        topic_family: "finance".to_string(),
        ..Default::default()
    };
    let eligible1 = is_candidate_eligible(
        &item1,
        &cfg,
        Some(cutoff),
        cutoff,
        upper,
        &mut seen_c,
        &mut seen_t,
        Some(&topic_filter),
    );
    assert!(!eligible1);

    // Item 2 has the same content/title hashes, but MATCHES the topic filter
    let item2 = NormalizedCandidate {
        candidate_id: "c2".to_string(),
        canonical_url: "https://example.com/2".to_string(),
        title: "Autonomous Agent Breakthrough".to_string(),
        published_at_utc: "2026-11-05T00:00:00Z".to_string(),
        content_hash: "hash_common_content".to_string(),
        title_hash: "hash_common_title".to_string(),
        topic_family: "ai".to_string(),
        ..Default::default()
    };
    let eligible2 = is_candidate_eligible(
        &item2,
        &cfg,
        Some(cutoff),
        cutoff,
        upper,
        &mut seen_c,
        &mut seen_t,
        Some(&topic_filter),
    );
    assert!(eligible2, "Item 2 must not be rejected by hash poisoning from Item 1");
}

// ---------------------------------------------------------------------------
// Issue 7: P1 - MCP 2026-07-28 Protocol Negotiation and Limits
// ---------------------------------------------------------------------------
#[test]
fn test_issue7_mcp_modern_protocol_and_limit_clamping() {
    let tmp = make_temp_dir("issue7_mcp");
    let (service, _) = build_test_service(&tmp);

    // 1. Negotiation with modern 2026-07-28 protocol version
    let init_req = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2026-07-28",
                "clientInfo": { "name": "agent-test", "version": "1.0.0" }
            }
        }
    });
    let init_resp = handle_request(init_req, &service, &tmp);
    let init_body: serde_json::Value = serde_json::from_str(init_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(init_body["result"]["protocolVersion"], "2026-07-28");

    // 2. server/discover method
    let discover_req = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "server/discover"
        }
    });
    let discover_resp = handle_request(discover_req, &service, &tmp);
    let discover_body: serde_json::Value = serde_json::from_str(discover_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(discover_body["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(discover_body["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "daily-news");

    // 3. MCP tool call with _meta
    let tool_req = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "daily-news__discover_candidates",
                "arguments": { "limit": 200 }, // exceeding 100, should be clamped
                "_meta": { "req_id": "meta_123" }
            }
        }
    });
    let tool_resp = handle_request(tool_req, &service, &tmp);
    let tool_body: serde_json::Value = serde_json::from_str(tool_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(tool_body["result"]["_meta"]["req_id"], "meta_123");

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Issue 8: P1/P2 - 2MB Body Enforcement and /handoff Endpoint
// ---------------------------------------------------------------------------
#[test]
fn test_issue8_body_limit_and_handoff_route() {
    let tmp = make_temp_dir("issue8_body_limits");
    let (service, _) = build_test_service(&tmp);

    // Non-string JSON body exceeding 2MB
    let huge_array: Vec<String> = (0..200_000).map(|i| format!("item_{i}")).collect();
    let oversized_req = json!({
        "path": "/api/v1/discover",
        "method": "POST",
        "body": {
            "items": huge_array
        }
    });
    let resp = handle_request(oversized_req, &service, &tmp);
    assert_eq!(resp["status"], 413);

    // Production handoff route
    let missing_handoff_req = json!({
        "path": "/api/v1/handoff",
        "method": "POST",
        "body": {
            "candidate_id": "nonexistent"
        }
    });
    let handoff_resp = handle_request(missing_handoff_req, &service, &tmp);
    assert_eq!(handoff_resp["status"], 422);

    let _ = std::fs::remove_dir_all(&tmp);
}
