//! Integration regression tests for v1.16.0 audit remediation:
//! - P0: Unified readiness gate: Raw data normalization cannot reach VIDEO_READY without resolvers
//! - P0: Trusted evidence provenance: Audit and visual mock artifacts yield MOCK_VERIFIED, NOT VIDEO_READY
//! - P1: Source revision invalidation: Candidate content changes invalidate stale research and transition to NEEDS_REVALIDATION
//! - P1: Transactional state: Consumed IDs and config updates leave RAM completely unmutated on disk failure

use daily_news::domain::{
    ClaimEvidence, EvidenceRef, NormalizedCandidate, RawNewsItem, ReadinessStatus,
    ResearchEvidenceFields, VerificationRecord, VisualEvidence,
};
use daily_news::ports::StoragePort;
use daily_news::research_readiness::evaluate_readiness_full;
use daily_news::storage::FileStorageAdapter;
use daily_news::url::{compute_sha256, compute_sha256_bytes};

#[test]
fn test_v116_raw_ingest_cannot_reach_video_ready_without_resolvers() {
    let mut raw = RawNewsItem::new(
        "raw_test_1",
        "https://example.com/system_news",
        "Deterministic Engine Benchmark Results",
        "2026-11-04T05:00:00Z",
    );
    raw.publisher = Some("TechWeekly".to_string());
    raw.content_kind = Some("news".to_string());
    raw.description = Some("System benchmarks achieve 4x latency reduction.".to_string());
    raw.body = Some("System benchmarks achieve 4x latency reduction across edge servers.".to_string());
    raw.content_origin = daily_news::domain::ContentOrigin::Article;

    let normalized = daily_news::discovery::normalize_raw_item(&raw, None, None)
        .expect("Normalize raw item");
    let readiness = normalized.research_readiness.as_ref().expect("Readiness evaluated");

    // Without resolvers and captured snapshot, raw ingest must hold at NeedsSource
    assert_ne!(readiness.status, ReadinessStatus::VideoReady);
    assert_eq!(readiness.status, ReadinessStatus::NeedsSource);
}

#[test]
fn test_v116_trusted_evidence_provenance_rejects_mock_from_video_ready() {
    let source_text = "Detailed whitepaper with reproducible empirical benchmarks achieving 4x latency reduction across systems.";
    let source_sha = compute_sha256(source_text);
    let primary_source = daily_news::domain::PrimarySourceEvidence {
        url: "https://example.com/whitepaper".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Deterministic Memory Whitepaper".to_string()),
        published_at: None,
        excerpt: Some(source_text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{source_sha}.txt")),
        snapshot_sha256: Some(source_sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    // 1. Mock Audit ("Benchmarking Laboratory") + Mock Visual ("PNG MOCK") -> MOCK_VERIFIED
    let mock_audit_text = "Official Audit Trail Report #2026-M1: Benchmarking Laboratory independently verified the performance benchmarks and confirmed findings for 4x latency reduction across all test runs.";
    let mock_audit_sha = compute_sha256(mock_audit_text);
    let mock_chart_bytes = b"PNG MOCK CHART ASSET DATA FOR PROVENANCE TEST";
    let mock_chart_sha = compute_sha256_bytes(mock_chart_bytes);

    let mock_claims = vec![ClaimEvidence {
        claim_id: "claim_mock_1".to_string(),
        statement: "4x latency reduction".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha.clone(),
            excerpt: "reproducible empirical benchmarks".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_mock_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "benchmarking_auditor".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(mock_audit_sha.clone()),
            secondary_source_ref: None,
        }),
    }];

    let mock_visuals = vec![VisualEvidence {
        asset_ref: "vis_mock_chart".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/mock_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(mock_chart_sha.clone()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let mock_audit_res = move |h: &str| -> Option<String> {
        if h == mock_audit_sha { Some(mock_audit_text.to_string()) } else { None }
    };
    let mock_asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == mock_chart_sha { Some(mock_chart_bytes.to_vec()) } else { None }
    };

    let readiness_mock = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &mock_claims,
        &mock_visuals,
        daily_news::domain::ContentOrigin::Article,
        None,
        Some(&mock_audit_res),
        Some(&mock_asset_res),
    );
    assert_eq!(readiness_mock.status, ReadinessStatus::MockVerified);
    assert_ne!(readiness_mock.status, ReadinessStatus::VideoReady);

    // 2. Trusted Accredited Issuer ("CertiK") + Real PNG Media Bytes -> VIDEO_READY
    let prod_audit_text = "Official Cryptographic Audit Trail Report from CertiK (https://certik.com/reports/2026-P1 ref #: 2026-P1): CertiK independently verified the performance benchmarks and confirmed findings for 4x latency reduction across all test runs.";
    let prod_audit_sha = compute_sha256(prod_audit_text);
    let prod_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let prod_png_sha = compute_sha256_bytes(prod_png_bytes);

    let prod_claims = vec![ClaimEvidence {
        claim_id: "claim_prod_1".to_string(),
        statement: "4x latency reduction".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: source_sha,
            excerpt: "reproducible empirical benchmarks".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "rec_prod_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "certik_senior_auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(prod_audit_sha.clone()),
            secondary_source_ref: Some("https://certik.com/reports/2026-P1".to_string()),
        }),
    }];

    let prod_visuals = vec![VisualEvidence {
        asset_ref: "vis_prod_chart".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/prod_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(prod_png_sha.clone()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let license_doc = "Copyright 2026 Vendor Corp (example.com). CC-BY Creative Commons license for commercial use of vis_prod_chart.";
    let prod_sec_res = move |r: &str| -> Option<String> {
        if r == "https://example.com/license" || r == compute_sha256("https://example.com/license") {
            Some(license_doc.to_string())
        } else {
            None
        }
    };

    let prod_audit_res = move |h: &str| -> Option<String> {
        if h == prod_audit_sha { Some(prod_audit_text.to_string()) } else { None }
    };
    let prod_asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == prod_png_sha { Some(prod_png_bytes.to_vec()) } else { None }
    };

    let readiness_prod = evaluate_readiness_full(
        Some(&primary_source),
        Some(source_text),
        &prod_claims,
        &prod_visuals,
        daily_news::domain::ContentOrigin::Article,
        Some(&prod_sec_res),
        Some(&prod_audit_res),
        Some(&prod_asset_res),
    );
    assert_eq!(readiness_prod.status, ReadinessStatus::VideoReady);
}

#[test]
fn test_v116_source_revision_invalidation_on_content_change() {
    let tmp = std::env::temp_dir().join(format!("test_invalidation_{}", uuid::Uuid::new_v4()));
    let storage = FileStorageAdapter::new(&tmp);

    let canonical = "https://example.com/article_to_update";
    let url_hash = daily_news::url::compute_sha256(canonical);
    let candidate_id = format!("cand_{}", &url_hash[..16]);

    // Initial candidate version 1
    let cand_v1 = NormalizedCandidate {
        candidate_id: candidate_id.clone(),
        canonical_url: canonical.to_string(),
        title: "Initial Article Version".to_string(),
        content_hash: "hash_version_1".to_string(),
        research: ResearchEvidenceFields {
            research_readiness: Some(daily_news::domain::ResearchReadiness {
                status: ReadinessStatus::VideoReady,
                reasons: vec!["Initial verification complete".to_string()],
                missing_fields: vec![],
                contract_version: "1.0.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    storage.save_candidates(&[cand_v1]).unwrap();

    // Re-ingest with modified body (new content_hash)
    let cand_v2 = NormalizedCandidate {
        candidate_id: candidate_id.clone(),
        canonical_url: canonical.to_string(),
        title: "Initial Article Version - Major Edit".to_string(),
        content_hash: "hash_version_2_edited".to_string(),
        research: ResearchEvidenceFields {
            research_readiness: Some(daily_news::domain::ResearchReadiness {
                status: ReadinessStatus::NeedsResearch,
                reasons: vec!["Fresh ingestion".to_string()],
                missing_fields: vec!["primary_source".to_string()],
                contract_version: "1.0.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    storage.save_candidates(&[cand_v2]).unwrap();

    let cached = storage.get_cached_candidates();
    let retrieved = cached
        .iter()
        .find(|c| c.candidate_id == candidate_id)
        .expect("Candidate found in storage");

    // Stale VIDEO_READY must be invalidated and transition to NEEDS_REVALIDATION
    assert_ne!(
        retrieved.research_readiness.as_ref().unwrap().status,
        ReadinessStatus::VideoReady,
        "Stale VIDEO_READY must NOT be retained after content revision changes"
    );
    assert_eq!(
        retrieved.research_readiness.as_ref().unwrap().status,
        ReadinessStatus::NeedsRevalidation
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_v116_transactional_persistence_unmutated_ram_on_disk_error() {
    let tmp = std::env::temp_dir().join(format!("test_tx_persist_{}", uuid::Uuid::new_v4()));
    let storage = FileStorageAdapter::new(&tmp);

    // Initial state
    assert_eq!(storage.consumed_count(), 0);
    let mut initial_cfg = storage.load_config();
    initial_cfg.freshness_window_hours = 42.0;
    storage.save_config(&initial_cfg).unwrap();
    assert_eq!(storage.load_config().freshness_window_hours, 42.0);

    // Sabotage disk write deterministically by replacing destination files with directories (causes fs::rename to fail with EISDIR/EPERM across all environments including root)
    let state_dir = daily_news::storage::resolve_state_dir(&tmp);
    let test_ids = vec!["cand_uncommitted_1".to_string(), "cand_uncommitted_2".to_string()];

    let consumed_path = state_dir.join("consumed.json");
    let _ = std::fs::remove_file(&consumed_path);
    std::fs::create_dir(&consumed_path).expect("Create obstruction directory for consumed.json");

    // 1. mark_consumed must return error AND leave in-memory consumed count strictly unchanged (0)
    let res_mark = storage.mark_consumed(&test_ids);
    assert!(res_mark.is_err(), "Atomic rename onto directory obstruction must fail");
    assert_eq!(
        storage.consumed_count(),
        0,
        "RAM state must NOT be mutated when disk write fails"
    );
    assert!(!storage.is_consumed("cand_uncommitted_1"));

    // Remove obstruction directory for clean config test
    let _ = std::fs::remove_dir(&consumed_path);

    // Sabotage config.json similarly
    let config_path = state_dir.join("config.json");
    let _ = std::fs::remove_file(&config_path);
    std::fs::create_dir(&config_path).expect("Create obstruction directory for config.json");

    // 2. save_config must return error AND leave in-memory config strictly unchanged (42.0)
    let mut new_cfg = initial_cfg.clone();
    new_cfg.freshness_window_hours = 999.0;
    let res_cfg = storage.save_config(&new_cfg);
    assert!(res_cfg.is_err(), "Config atomic rename onto directory obstruction must fail");
    assert_eq!(
        storage.load_config().freshness_window_hours,
        42.0,
        "RAM config must NOT be mutated when disk write fails"
    );

    let _ = std::fs::remove_dir(&config_path);
    let _ = std::fs::remove_dir_all(&tmp);
}
