//! Comprehensive regression test suite for v1.19.0 audit remediation:
//! 1. P0: Trusted audit provenance vs content matching (ISSUER_AUTHENTICATED vs AUDIT_CONTENT_MATCHED).
//! 2. P0: Strict license rights verification requiring resolved document, asset linkage, and commercial scope.
//! 3. P1: Real media decoding (structural container, chunk, and frame header parsing).
//! 4. P1: Cache revision merging in get_candidates and unified CandidateEligibilityPolicy in try_cache_tier.
//! 5. P1: Atomic production handoff claim, consumed rejection, and freshness enforcement.
//! 6. P1: Discovery admission for feed-observed articles with deferred handoff freshness gate.
//! 7. P1: MCP 2026-07-28 stateless protocol conformance, version validation, and verify_production_handoff tool schema.

use daily_news::discovery::CandidateEligibilityPolicy;
use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, ModuleConfig, NormalizedCandidate,
    PrimarySourceEvidence, ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::handlers::handle_request;
use daily_news::mcp_schema::get_mcp_tools_list;
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    check_audit_provenance_level, evaluate_readiness_full, is_audit_source_verified,
    is_valid_visual_asset_format, is_visual_media_decodable, is_visual_rights_verified,
    AuditProvenanceStatus,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::{extract_html_published_at, HttpSourceCaptureAdapter};
use daily_news::storage::FileStorageAdapter;
use daily_news::url::{compute_sha256, compute_sha256_bytes};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

mod common;
use common::{make_temp_dir, make_valid_gif, make_valid_jpeg, MockFeedFetcher};

// ---------------------------------------------------------------------------
// Issue 1: P0 - Audit Provenance Authentication (Negative & Positive Tests)
// ---------------------------------------------------------------------------
#[test]
fn test_issue1_audit_provenance_issuer_authenticated_vs_content_matched() {
    let source_text = "Benchmarking verified 4x throughput improvement on mainnet cluster.";
    let source_sha = compute_sha256(source_text);

    let primary_source = PrimarySourceEvidence {
        url: "https://example.com/audit_test".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("TIER_1".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Audit Test".to_string()),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        excerpt: Some(source_text.to_string()),
        snapshot_ref: Some(format!("{source_sha}.txt")),
        snapshot_sha256: Some(source_sha.clone()),
        ..Default::default()
    };

    // Case 1A: Fabricated wording claiming CertiK without authoritative domain or signature
    let fabricated_text = "Official Cryptographic Audit Trail Report #2026-CERT-01: CertiK independently verified findings confirmed for 4x throughput improvement on mainnet cluster.";
    let fab_sha = compute_sha256(fabricated_text);
    let fabricated_rec = VerificationRecord {
        record_id: "rec_fab_01".to_string(),
        verified_at: "2026-11-04T05:00:00Z".to_string(),
        reviewer_id: "certik_senior_auditor".to_string(),
        organization: "CertiK".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: Some(fab_sha.clone()),
        secondary_source_ref: None,
    };
    let fab_claim = ClaimEvidence {
        claim_id: "c_fab_1".to_string(),
        statement: "4x throughput improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha.clone(),
            excerpt: "4x throughput improvement".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(fabricated_rec.clone()),
    };

    // Must be classified as AuditContentMatched only, NOT IssuerAuthenticated
    assert!(!is_audit_source_verified(fabricated_text, &fabricated_rec));
    assert_eq!(
        check_audit_provenance_level(fabricated_text, &fabricated_rec, &fab_claim),
        AuditProvenanceStatus::AuditContentMatched
    );

    // Case 1B: Attacker domain spoofing (not in accredited domains for CertiK)
    let spoofed_text = "Official Report: CertiK verified findings confirmed for 4x throughput improvement at https://attacker.com/fake-certik ref #: 2026-01.";
    let spoofed_rec = VerificationRecord {
        secondary_source_ref: Some("https://attacker.com/fake-certik".to_string()),
        ..fabricated_rec.clone()
    };
    assert!(!is_audit_source_verified(spoofed_text, &spoofed_rec));

    // Case 1C: Genuine CertiK audit anchored with authoritative domain and report id
    let base_audit = "Official Audit Report from CertiK: independently verified findings confirmed for 4x throughput improvement (https://certik.com/reports/2026-CERT-01 ref #: 2026-CERT-01).";
    let base_sha = compute_sha256(base_audit);
    let att = daily_news::research_readiness::make_test_certik_attestation("https://certik.com/reports/2026-CERT-01", &base_sha);
    let authentic_text = format!("{base_audit}\n{att}");
    let auth_sha = compute_sha256(&authentic_text);
    let authentic_rec = VerificationRecord {
        record_id: "rec_auth_01".to_string(),
        verified_at: "2026-11-04T05:00:00Z".to_string(),
        reviewer_id: "certik_senior_auditor".to_string(),
        organization: "CertiK".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: Some(auth_sha.clone()),
        secondary_source_ref: Some("https://certik.com/reports/2026-CERT-01".to_string()),
    };
    let auth_claim = ClaimEvidence {
        verification_record: Some(authentic_rec.clone()),
        ..fab_claim.clone()
    };

    let certik_resolved_report = "Official CertiK Benchmark Audit Report 2026-CERT-01. Verified 4x throughput improvement confirmed.";
    let license_doc = "Copyright 2026 Example Corp. Licensed under CC-BY for commercial use of vis_auth_1.";

    let _sec_res_unresolved = move |r: &str| -> Option<String> {
        if r == "https://example.com/license" || r == compute_sha256("https://example.com/license") {
            Some(license_doc.to_string())
        } else {
            None
        }
    };

    let sec_res_authenticated = move |r: &str| -> Option<String> {
        if r == "https://example.com/license" || r == compute_sha256("https://example.com/license") {
            Some(license_doc.to_string())
        } else if r == "https://certik.com/reports/2026-CERT-01" || r == compute_sha256("https://certik.com/reports/2026-CERT-01") {
            Some(certik_resolved_report.to_string())
        } else {
            None
        }
    };

    // Case 1D: Even if audit_text mentions https://certik.com, without secondary_source_ref on accredited domain it fails closed as AuditContentMatched only
    let unanchored_rec = VerificationRecord {
        secondary_source_ref: None,
        ..authentic_rec.clone()
    };
    assert_eq!(
        daily_news::research_readiness::check_audit_provenance_level(
            &authentic_text,
            &unanchored_rec,
            &auth_claim
        ),
        AuditProvenanceStatus::AuditContentMatched
    );

    // Case 1E: With secondary_source_ref resolved via secondary_resolver without attestation -> ResolvedDocument
    let plain_authentic_text = "Official Audit Report from CertiK: independently verified findings confirmed for 4x throughput improvement (https://certik.com/reports/2026-CERT-01 ref #: 2026-CERT-01).";
    assert_eq!(
        daily_news::research_readiness::check_audit_provenance_level_with_resolver(
            plain_authentic_text,
            &authentic_rec,
            &auth_claim,
            Some(&sec_res_authenticated)
        ),
        AuditProvenanceStatus::ResolvedDocument
    );

    // Case 1E2: Accompanied by authentic trusted fetch attestation envelope -> IssuerAuthenticated
    assert_eq!(
        daily_news::research_readiness::check_audit_provenance_level_with_resolver(
            &authentic_text,
            &authentic_rec,
            &auth_claim,
            Some(&sec_res_authenticated)
        ),
        AuditProvenanceStatus::IssuerAuthenticated
    );

    // Case 1F: Resolver returning None MUST fail closed to AuditContentMatched
    assert_eq!(
        daily_news::research_readiness::check_audit_provenance_level_with_resolver(
            &authentic_text,
            &authentic_rec,
            &auth_claim,
            Some(&_sec_res_unresolved)
        ),
        AuditProvenanceStatus::AuditContentMatched
    );

    // Case 1G: Without secondary_resolver (None), domain doc cannot be resolved -> AuditContentMatched
    assert_eq!(
        daily_news::research_readiness::check_audit_provenance_level(
            &authentic_text,
            &authentic_rec,
            &auth_claim
        ),
        AuditProvenanceStatus::AuditContentMatched
    );

    // Valid PNG asset & license resolver
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let valid_png_sha = compute_sha256_bytes(valid_png_bytes);
    let valid_visuals = vec![VisualEvidence {
        asset_ref: "vis_auth_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(valid_png_sha.clone()),
        license_status: Some("cc-by".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
        ..Default::default()
    }];

    let asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == valid_png_sha { Some(valid_png_bytes.to_vec()) } else { None }
    };
    let fab_audit_res = move |h: &str| -> Option<String> {
        if h == fab_sha { Some(fabricated_text.to_string()) } else { None }
    };
    let auth_audit_res = move |h: &str| -> Option<String> {
        if h == auth_sha { Some(authentic_text.to_string()) } else { None }
    };

    // Fabricated audit fails closed with NeedsVerification
    let res_fab = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &[fab_claim],
        &valid_visuals,
        ContentOrigin::Article,
        Some(&sec_res_authenticated),
        Some(&fab_audit_res),
        Some(&asset_res),
    );
    assert_eq!(res_fab.status, ReadinessStatus::NeedsVerification);

    // Authentic audit with provenance reaches VideoReady
    let res_auth = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &[auth_claim],
        &valid_visuals,
        ContentOrigin::Article,
        Some(&sec_res_authenticated),
        Some(&auth_audit_res),
        Some(&asset_res),
    );
    assert_eq!(res_auth.status, ReadinessStatus::VideoReady);
}

// ---------------------------------------------------------------------------
// Issue 2: P0 - Strict License Rights Verification
// ---------------------------------------------------------------------------
#[test]
fn test_issue2_strict_license_rights_verification() {
    let mut visual = VisualEvidence {
        asset_ref: "chart_perf".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart_perf.png".to_string(),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()),
        license_status: Some("cc-by".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
        ..Default::default()
    };

    // Case 2A: secondary_resolver is None -> MUST return false (fallback eliminated!)
    assert!(!is_visual_rights_verified(&visual, None));

    // Case 2B: resolver returns None (document unresolved) -> MUST return false
    let empty_resolver = |_r: &str| -> Option<String> { None };
    assert!(!is_visual_rights_verified(&visual, Some(&empty_resolver)));

    // Case 2C: resolved document has prohibited non-commercial restriction -> MUST return false
    let non_commercial_doc = "Copyright 2026 Author. Licensed for non-commercial personal use only. All rights reserved for commercial distribution.";
    let nc_resolver = move |_r: &str| -> Option<String> { Some(non_commercial_doc.to_string()) };
    assert!(!is_visual_rights_verified(&visual, Some(&nc_resolver)));

    // Case 2D: resolved document does not reference the asset -> MUST return false
    let unrelated_doc = "Copyright 2026 Company. Grant of MIT license for unrelated_asset.png only.";
    let unrelated_resolver = move |_r: &str| -> Option<String> { Some(unrelated_doc.to_string()) };
    assert!(!is_visual_rights_verified(&visual, Some(&unrelated_resolver)));

    // Case 2E: valid affirmative commercial grant bound to asset and licensor -> MUST return true
    let valid_doc = "Copyright 2026 Acme Corp. The author grants permission for commercial use and distribution of chart_perf under CC-BY.";
    let valid_resolver = move |_r: &str| -> Option<String> { Some(valid_doc.to_string()) };
    assert!(is_visual_rights_verified(&visual, Some(&valid_resolver)));

    // Case 2F: verified_rights is false -> MUST return false even with valid document
    visual.verified_rights = Some(false);
    assert!(!is_visual_rights_verified(&visual, Some(&valid_resolver)));
}

// ---------------------------------------------------------------------------
// Issue 3: P1 - Real Media Decoding
// ---------------------------------------------------------------------------
#[test]
fn test_issue3_real_media_decoding_containers() {
    // Valid PNG: 68 bytes complete with IHDR, IDAT (valid zlib Deflate), IEND
    let valid_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    assert!(is_valid_visual_asset_format(valid_png));
    assert!(is_visual_media_decodable(valid_png));

    // Corrupted PNG: Has bytes "IDAT" and "IEND" substring but truncated before zlib/IEND chunk
    let fake_marker_png = b"\x89PNG\r\n\x1a\nIDATIENDfakefakefakefakefakefakefakefakefake";
    assert!(!is_visual_media_decodable(fake_marker_png));

    // Fake 59-byte PNG with fake IDAT and corrupt compression stream
    let fake_59b_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATfakeidatdata\0\0\0\0IEND\xaeB`\x82";
    assert!(!is_visual_media_decodable(fake_59b_png));

    // PNG with invalid zlib header in IDAT
    let corrupt_zlib_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDAT\xFF\xFF\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    assert!(!is_visual_media_decodable(corrupt_zlib_png));

    // Valid JPEG: complete 1x1 JFIF image decodable by raster engine
    let valid_jpeg = make_valid_jpeg();
    assert!(is_visual_media_decodable(&valid_jpeg));

    // Corrupted JPEG: Missing SOF dimensions
    let corrupt_jpeg = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xD9];
    assert!(!is_visual_media_decodable(corrupt_jpeg));

    // Valid GIF89a with GCT (1x1) + Image Descriptor (1x1) + LZW + Trailer (3B)
    let valid_gif = make_valid_gif();
    assert!(is_visual_media_decodable(&valid_gif));

    // Corrupted GIF: Contains byte 0x3B but has zero dimensions
    let corrupt_gif = &[b'G', b'I', b'F', b'8', b'9', b'a', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3B];
    assert!(!is_visual_media_decodable(corrupt_gif));
}

// ---------------------------------------------------------------------------
// Issue 4: P1 - Cache Revision Merging and Unified Policy
// ---------------------------------------------------------------------------
#[test]
fn test_issue4_cache_revision_merging_and_unified_try_cache_tier() {
    let tmp = make_temp_dir("issue4");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let scorer = CandidateScorer::default();

    let cand_id = format!("cand_{}", &compute_sha256("https://example.com/rev_test")[..16]);
    let now_utc = daily_news::date::current_utc_timestamp();
    // Initial candidate cached with older content
    let candidate_v1 = NormalizedCandidate {
        candidate_id: cand_id.clone(),
        canonical_url: "https://example.com/rev_test".to_string(),
        title: "Initial Title".to_string(),
        publisher: "example.com".to_string(),
        published_at_utc: now_utc.clone(),
        content_hash: compute_sha256("content v1"),
        title_hash: compute_sha256("initial title"),
        content_origin: ContentOrigin::Article,
        ..Default::default()
    };
    storage.save_candidates(&[candidate_v1]).unwrap();

    // Live raw item with same URL / candidate_id but updated content v2
    let updated_raw = daily_news::domain::RawNewsItem {
        id: "https://example.com/rev_test".to_string(),
        url: "https://example.com/rev_test".to_string(),
        title: "Updated Title Across Revision".to_string(),
        published_at: now_utc,
        publisher: Some("example.com".to_string()),
        author: None,
        description: Some("New content revision".to_string()),
        body: Some("content v2 with updated facts".to_string()),
        discussion_url: None,
        tags: vec![],
        is_evergreen: false,
        content_kind: Some("article".to_string()),
        updated_at: None,
        evidence: Default::default(),
        extra: Default::default(),
        content_origin: ContentOrigin::Article,
        published_at_kind: daily_news::domain::PublishedAtKind::ArticlePublication,
        research: Default::default(),
    };

    let fetcher = Arc::new(MockFeedFetcher { items: vec![updated_raw] });
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

    // get_candidates MUST merge incoming revision rather than silently ignoring it
    let candidates = service.get_candidates(10);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].candidate_id, cand_id);
    assert_eq!(candidates[0].title, "Updated Title Across Revision");
    assert_ne!(candidates[0].content_hash, compute_sha256("content v1"));

    // try_cache_tier via discover enforces CandidateEligibilityPolicy uniformly
    let disc_res = service.discover(10, None, None, None, None, None).unwrap();
    assert_eq!(disc_res.status, "ELIGIBLE");
    assert_eq!(disc_res.candidates.len(), 1);

    // CRITICAL: Verify merged revision survives restart of storage adapter
    let restarted = FileStorageAdapter::new(&tmp);
    let recovered = restarted.get_cached_candidates();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].candidate_id, cand_id);
    assert_eq!(recovered[0].title, "Updated Title Across Revision");
    assert_ne!(recovered[0].content_hash, compute_sha256("content v1"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Issue 5: P1 - Atomic Handoff Claim, Consumed Check & Freshness Gate
// ---------------------------------------------------------------------------
#[test]
fn test_issue5_atomic_handoff_claim_and_freshness_gate() {
    let tmp = make_temp_dir("issue5");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp));
    let scorer = CandidateScorer::default();
    let fetcher = Arc::new(MockFeedFetcher { items: vec![] });
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer).with_source_capture(capturer);

    let state_dir = tmp.join(".state");
    let evidence_dir = state_dir.join("evidence");
    let assets_dir = state_dir.join("assets");
    std::fs::create_dir_all(&evidence_dir).unwrap();
    std::fs::create_dir_all(&assets_dir).unwrap();

    let primary_text = "Verified primary report: 4x speedup on neural compiler benchmarks.";
    let primary_sha = compute_sha256(primary_text);
    std::fs::write(evidence_dir.join(format!("{primary_sha}.txt")), primary_text).unwrap();

    let base_audit = "Official Audit Report from CertiK: verified findings confirmed for 4x speedup on neural compiler benchmarks (https://certik.com/reports/2026-HND ref #: 2026-HND).";
    let base_sha = compute_sha256(base_audit);
    let att = daily_news::research_readiness::make_test_certik_attestation("https://certik.com/reports/2026-HND", &base_sha);
    let audit_text = format!("{base_audit}\n{att}");
    let audit_sha = compute_sha256(&audit_text);
    std::fs::write(evidence_dir.join(format!("{audit_sha}.txt")), &audit_text).unwrap();

    let certik_report = "Official CertiK Benchmark Audit Report 2026-HND. Verified 4x speedup on neural compiler benchmarks.";
    let certik_url_sha = compute_sha256("https://certik.com/reports/2026-HND");
    std::fs::write(evidence_dir.join(format!("{certik_url_sha}.txt")), certik_report).unwrap();

    let license_text = "Copyright 2026 Vendor Corp (example.com). CC-BY Creative Commons license for commercial use of asset_hnd.";
    let lic_url_sha = compute_sha256("https://example.com/license");
    std::fs::write(evidence_dir.join(format!("{lic_url_sha}.txt")), license_text).unwrap();

    let png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let png_sha = compute_sha256_bytes(png_bytes);
    std::fs::write(assets_dir.join(format!("{png_sha}.bin")), png_bytes).unwrap();

    let now_ts = daily_news::date::current_utc_timestamp();
    let valid_candidate = NormalizedCandidate {
        candidate_id: "cand_handoff_01".to_string(),
        canonical_url: "https://example.com/hnd_01".to_string(),
        title: "Speedup".to_string(),
        publisher: "example.com".to_string(),
        published_at_utc: now_ts.clone(),
        content_hash: compute_sha256("content 01"),
        title_hash: compute_sha256("speedup"),
        content_origin: ContentOrigin::Article,
        research: daily_news::domain::ResearchEvidenceFields {
            primary_source: Some(PrimarySourceEvidence {
                url: "https://example.com/hnd_01".to_string(),
                fetch_status: "SUCCESS".to_string(),
                published_at: Some(now_ts),
                snapshot_ref: Some(format!("{primary_sha}.txt")),
                snapshot_sha256: Some(primary_sha.clone()),
                ..Default::default()
            }),
            claims: vec![ClaimEvidence {
                claim_id: "c_hnd_1".to_string(),
                statement: "4x speedup on neural compiler".to_string(),
                evidence_refs: vec![EvidenceRef {
                    source_ref: primary_sha.clone(),
                    excerpt: "4x speedup on neural compiler".to_string(),
                    locator: Some("p:1".to_string()),
                }],
                verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
                reviewer: None,
                verification_record: Some(VerificationRecord {
                    record_id: "rec_hnd_1".to_string(),
                    verified_at: "2026-11-04T00:00:00Z".to_string(),
                    reviewer_id: "certik_auditor".to_string(),
                    organization: "CertiK".to_string(),
                    method: "reproducible_benchmark".to_string(),
                    audit_trail_ref: Some(audit_sha.clone()),
                    secondary_source_ref: Some("https://certik.com/reports/2026-HND".to_string()),
                }),
            }],
            visual_evidence: vec![VisualEvidence {
                asset_ref: "asset_hnd".to_string(),
                kind: "chart".to_string(),
                source_url: "https://example.com/chart.png".to_string(),
                provenance_status: Some("verified".to_string()),
                checksum_sha256: Some(png_sha.clone()),
                license_status: Some("cc-by".to_string()),
                license_evidence_ref: Some("https://example.com/license".to_string()),
                verified_rights: Some(true),
                ..Default::default()
            }],
            research_readiness: Some(daily_news::domain::ResearchReadiness {
                status: ReadinessStatus::VideoReady,
                reasons: vec!["Ready".to_string()],
                missing_fields: vec![],
                contract_version: "2.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    storage.save_candidates(std::slice::from_ref(&valid_candidate)).unwrap();

    // First handoff succeeds and claims candidate atomically
    let handoff_1 = service.verify_production_handoff("cand_handoff_01");
    if let Err(ref e) = handoff_1 {
        panic!("HANDOFF 1 ERR: {:?}", e);
    }
    assert!(handoff_1.is_ok());
    let handoff_obj = handoff_1.unwrap();
    let reservation_token = handoff_obj.reservation_token.clone();

    // Storage confirms candidate is now marked consumed
    assert!(storage.is_consumed("cand_handoff_01"));

    // Concurrent / duplicate worker handoff fails closed with ALREADY_CONSUMED
    let handoff_duplicate = service.verify_production_handoff("cand_handoff_01");
    assert!(handoff_duplicate.is_err());
    let (code, msg) = handoff_duplicate.err().unwrap();
    assert_eq!(code, "ALREADY_CONSUMED");
    assert!(msg.contains("already been consumed") || msg.contains("actively reserved"));

    // Stale candidate (> max_age_days) fails with STALE_CANDIDATE
    let mut stale_candidate = valid_candidate.clone();
    stale_candidate.candidate_id = "cand_stale_01".to_string();
    stale_candidate.published_at_utc = "2020-01-01T00:00:00+00:00".to_string();
    stale_candidate.updated_at = Some("2020-01-01T00:00:00+00:00".to_string());
    storage.save_candidates(&[stale_candidate]).unwrap();

    let handoff_stale = service.verify_production_handoff("cand_stale_01");
    assert!(handoff_stale.is_err());
    let (code_stale, _) = handoff_stale.err().unwrap();
    assert_eq!(code_stale, "STALE_CANDIDATE");

    // GET /handoff must be rejected with 405 Method Not Allowed
    let req_get = json!({
        "path": "/api/v1/handoff",
        "method": "GET",
        "body": { "candidate_id": "cand_handoff_01" }
    });
    let resp_get = handle_request(req_get, &service, &tmp);
    assert_eq!(resp_get["status"], 405);

    // Reservation release lifecycle:
    // 1. Release with wrong token fails
    assert!(service.release_handoff("cand_handoff_01", "wrong_token").is_err());
    assert!(storage.is_consumed("cand_handoff_01"));

    // 2. Release with valid reservation token succeeds and returns candidate to available
    assert!(service.release_handoff("cand_handoff_01", &reservation_token).is_ok());
    assert!(!storage.is_consumed("cand_handoff_01"));

    // 3. Re-reserve and commit handoff lifecycle
    let handoff_re = service.verify_production_handoff("cand_handoff_01").expect("Must re-reserve");
    let re_token = handoff_re.reservation_token.clone();

    // Commit handoff moves to PUBLISHED
    assert!(service.commit_handoff("cand_handoff_01", &re_token).is_ok());
    assert!(storage.is_consumed("cand_handoff_01"));

    // Once PUBLISHED, release is rejected
    assert!(service.release_handoff("cand_handoff_01", &re_token).is_err());

    let _ = std::fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Issue 6: P1 - Feed-observed News with Deferred Handoff Freshness Gate
// ---------------------------------------------------------------------------
#[test]
fn test_issue6_feed_observed_discovery_admission_and_html_date_extraction() {
    let cfg = ModuleConfig {
        freshness_window_hours: 48.0,
        max_age_days: 7,
        require_latest_news: true,
        ..Default::default()
    };
    let policy = CandidateEligibilityPolicy::from_config(&cfg, None, None, None);

    // Article discovered from Reddit feed without verified primary source published_at
    let unverified_pub_article = NormalizedCandidate {
        candidate_id: "cand_reddit_feed_01".to_string(),
        canonical_url: "https://example.com/reddit_news".to_string(),
        title: "Fresh Reddit Article".to_string(),
        publisher: "reddit.com".to_string(),
        published_at_utc: "2026-11-05T00:00:00+00:00".to_string(),
        content_hash: compute_sha256("reddit content"),
        title_hash: compute_sha256("fresh reddit article"),
        content_origin: ContentOrigin::Article,
        published_at_kind: "feed_observed".to_string(),
        research: daily_news::domain::ResearchEvidenceFields {
            primary_source: Some(PrimarySourceEvidence {
                url: "https://example.com/reddit_news".to_string(),
                fetch_status: "SUCCESS".to_string(),
                published_at: None, // unverified at discovery time
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let mut seen_content = HashSet::new();
    let mut seen_titles = HashSet::new();

    // MUST be admitted into research queue based on feed observation time
    assert!(policy.is_candidate_eligible(
        &unverified_pub_article,
        &mut seen_content,
        &mut seen_titles
    ));

    // HTML Date extraction verifies published_at from standard meta and time tags
    let html_meta = r#"<html><head><meta property="article:published_time" content="2026-11-05T08:30:00Z"></head><body>Text</body></html>"#;
    assert_eq!(
        extract_html_published_at(html_meta),
        Some("2026-11-05T08:30:00Z".to_string())
    );

    let html_time = r#"<html><body><time datetime="2026-11-04T19:00:00Z">Nov 4</time></body></html>"#;
    assert_eq!(
        extract_html_published_at(html_time),
        Some("2026-11-04T19:00:00Z".to_string())
    );

    // Unicode HTML metadata date extraction without char boundary panic
    let html_unicode = r#"<html><head><title>Unicode üöäß</title><meta property="article:published_time" content="2026-11-05T08:30:00Z"></head><body>Café Straße München äöü ß</body></html>"#;
    assert_eq!(
        extract_html_published_at(html_unicode),
        Some("2026-11-05T08:30:00Z".to_string())
    );
}

// ---------------------------------------------------------------------------
// Issue 7: P1 - MCP 2026-07-28 Interoperability and Tool Schema
// ---------------------------------------------------------------------------
#[test]
fn test_issue7_mcp_stateless_protocol_and_verify_production_handoff_schema() {
    let tmp = make_temp_dir("issue7");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let scorer = CandidateScorer::default();
    let fetcher = Arc::new(MockFeedFetcher { items: vec![] });
    let service = DailyNewsService::new(fetcher, storage, scorer);

    // 1. tools/list exposes verify_production_handoff
    let tools = get_mcp_tools_list();
    let handoff_tool = tools
        .iter()
        .find(|t| t["name"] == "verify_production_handoff")
        .expect("verify_production_handoff must be declared in MCP tools list");
    assert_eq!(
        handoff_tool["inputSchema"]["required"],
        json!(["candidate_id"])
    );

    // 2. Request with unsupported protocolVersion in _meta returns JSON-RPC error -32602
    let req_invalid_ver = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "_meta": { "protocolVersion": "9999-99-99" }
        }
    });
    let resp_invalid = handle_request(req_invalid_ver, &service, &tmp);
    let body_invalid: Value = serde_json::from_str(resp_invalid["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_invalid["error"]["code"], -32602);
    assert!(body_invalid["error"]["message"].as_str().unwrap().contains("Unsupported MCP protocolVersion"));

    // 3. Stateless MCP 2026-07-28 request dispatches without prior initialize call
    let req_stateless = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "_meta": { "protocolVersion": "2026-07-28" }
        }
    });
    let resp_stateless = handle_request(req_stateless, &service, &tmp);
    assert_eq!(resp_stateless["status"], 200);
    let body_stateless: Value = serde_json::from_str(resp_stateless["body"].as_str().unwrap()).unwrap();
    assert!(body_stateless["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "verify_production_handoff"));

    // 4. Request with canonical MCP 2026-07-28 metadata key "io.modelcontextprotocol/protocolVersion"
    let req_canonical = json!({
        "path": "/mcp",
        "method": "POST",
        "body": {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/list",
            "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" }
        }
    });
    let resp_canonical = handle_request(req_canonical, &service, &tmp);
    assert_eq!(resp_canonical["status"], 200);

    let _ = std::fs::remove_dir_all(&tmp);
}
