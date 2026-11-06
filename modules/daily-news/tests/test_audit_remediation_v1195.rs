//! Integration and regression test suite for Daily News v1.19.5 audit remediation:
//!
//! 1. P0: Cryptographic Signed Attestation (TrustStore, non-hardcoded secrets, Ed25519/HMAC-SHA256, SSRF/expiry checks)
//! 2. P0: Transactional SQLite Storage (ACID transactions, WAL, concurrent worker locking, JSON array migration)
//! 3. P0: Invariant Defense: Schema migration marker & PUBLISHED cannot be downgraded to RESERVED on restart
//! 4. P1: Durability with PRAGMA synchronous = FULL
//! 5. P1: Strict License-Type Matching (CC-BY vs CC-BY-SA mutual exclusion)
//! 6. P1: Real Media Raster Image Decoding & Mock Media Rejection in VIDEO_READY Gates

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, HandoffReservation, NormalizedCandidate,
    PrimarySourceEvidence, ReadinessStatus, ReservationStatus, VerificationRecord, VisualEvidence,
};
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    create_signed_gateway_attestation, decode_raster_image, evaluate_readiness_full,
    global_trust_store, is_audit_source_verified_with_resolver, is_visual_media_decodable,
    is_visual_rights_verified, AuditProvenanceStatus, TrustStore, TrustedKey,
};
use daily_news::storage::SqliteStorageAdapter;
use daily_news::url::{compute_sha256, compute_sha256_bytes};

fn make_temp_dir(test_name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("dn_test_v1195_{}_{}", test_name, uuid::Uuid::new_v4()));
    let _ = fs::create_dir_all(&path);
    path
}

// ===========================================================================
// 1. Cryptographically Verified Attestation & Trust Store (P0)
// ===========================================================================
#[test]
fn test_cryptographic_attestation_trust_store_and_tamper_rejection() {
    let audit_text = "Independent benchmark formal verification audit conducted and verified by CertiK confirms zero vulnerabilities across consensus layer. Audit findings verified by CertiK.";
    let audit_sha = compute_sha256(audit_text);
    let sec_url = "https://certik.com/reports/certik_audit_consensus_2026";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let rec = VerificationRecord {
        record_id: "certik_consensus_2026".to_string(),
        verified_at: "2026-11-06T00:00:00Z".to_string(),
        reviewer_id: "certik_lead_auditor".to_string(),
        organization: "CertiK".to_string(),
        method: "formal_verification".to_string(),
        audit_trail_ref: Some(audit_sha.clone()),
        secondary_source_ref: Some(sec_url.to_string()),
    };
    let claim = ClaimEvidence {
        claim_id: "c_cons_01".to_string(),
        statement: "Zero consensus vulnerabilities".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: audit_sha.clone(),
            excerpt: "zero vulnerabilities across consensus layer".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(rec.clone()),
    };

    let resolver = move |_r: &str| -> Option<String> {
        Some("Formal verification report issued by CertiK for certik_consensus_2026. Security assessment verified.".to_string())
    };

    // Subtest A: Attestation forged with source-known hardcoded secret against a fresh TrustStore MUST FAIL
    let clean_store = TrustStore::new();
    clean_store.seed_accredited_keys(); // No HMAC secret hardcoded!
    let forged_source_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        &audit_sha,
        now,
        "CertiK",
        "gateway-internal-hmac-1",
        b"gateway-production-attestation-secret-2026",
    );
    let forged_doc = format!("{audit_text}\n{forged_source_attestation}");
    let parsed_forged = daily_news::research_readiness::parse_attestation_envelope(&forged_doc)
        .expect("Parsed envelope");
    assert!(
        !daily_news::research_readiness::verify_parsed_attestation(
            &parsed_forged,
            sec_url,
            Some(&audit_sha),
            None,
            &clean_store,
        ),
        "Forged attestation created with source-known secret MUST fail closed in production TrustStore"
    );

    // Subtest B: Explicitly configured secret in TrustStore succeeds
    let test_secret = b"properly-configured-production-secret-999";
    let key_id = "configured-gw-key-1";
    clean_store.register_key(key_id, TrustedKey::HmacSha256(test_secret.to_vec()));
    global_trust_store().register_key(key_id, TrustedKey::HmacSha256(test_secret.to_vec()));

    let authentic_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        &audit_sha,
        now,
        "CertiK",
        key_id,
        test_secret,
    );
    let authentic_doc = format!("{audit_text}\n{authentic_attestation}");
    let authentic_sha = compute_sha256(&authentic_doc);
    let rec_auth = VerificationRecord {
        audit_trail_ref: Some(authentic_sha),
        ..rec.clone()
    };
    let claim_auth = ClaimEvidence {
        verification_record: Some(rec_auth.clone()),
        ..claim.clone()
    };

    assert!(
        is_audit_source_verified_with_resolver(&authentic_doc, &rec_auth, Some(&resolver)),
        "Authentic signed attestation with configured secret must pass source verification"
    );
    let status = daily_news::research_readiness::check_audit_provenance_level_with_resolver(
        &authentic_doc,
        &rec_auth,
        &claim_auth,
        Some(&resolver),
    );
    assert_eq!(
        status,
        AuditProvenanceStatus::IssuerAuthenticated,
        "Authentic signed attestation must achieve IssuerAuthenticated"
    );

    // Subtest C: Tampered content hash in attestation must FAIL closed
    let tampered_hash = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let tampered_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        tampered_hash,
        now,
        "CertiK",
        key_id,
        test_secret,
    );
    let tampered_doc = format!("{audit_text}\n{tampered_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&tampered_doc, &rec, Some(&resolver)),
        "Tampered content hash in attestation must fail closed"
    );

    // Subtest D: Malicious redirect to localhost or private IP in Final-URL must FAIL closed
    let ssrf_redirect_attestation = create_signed_gateway_attestation(
        sec_url,
        "http://127.0.0.1:8080/internal_secrets",
        &audit_sha,
        now,
        "CertiK",
        key_id,
        test_secret,
    );
    let ssrf_doc = format!("{audit_text}\n{ssrf_redirect_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&ssrf_doc, &rec, Some(&resolver)),
        "SSRF redirect in attestation must fail closed"
    );

    // Subtest E: Expired attestation (> 180 days TTL) must FAIL closed
    let expired_time = now.saturating_sub(200 * 86400);
    let expired_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        &audit_sha,
        expired_time,
        "CertiK",
        key_id,
        test_secret,
    );
    let expired_doc = format!("{audit_text}\n{expired_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&expired_doc, &rec, Some(&resolver)),
        "Expired attestation older than 180 days must fail closed"
    );

    // Subtest F: Future attestation (> 3600s clock skew) must FAIL closed
    let future_time = now + 7200;
    let future_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        &audit_sha,
        future_time,
        "CertiK",
        key_id,
        test_secret,
    );
    let future_doc = format!("{audit_text}\n{future_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&future_doc, &rec, Some(&resolver)),
        "Future attestation beyond clock skew must fail closed"
    );
}

// ===========================================================================
// 2. Transactional SQLite Storage & Concurrency (P0)
// ===========================================================================
#[test]
fn test_sqlite_transactional_storage_concurrency_and_atomicity() {
    let tmp = make_temp_dir("sqlite_concurrency");
    let storage = Arc::new(SqliteStorageAdapter::new(&tmp).expect("Init SqliteStorageAdapter"));

    let candidate = NormalizedCandidate {
        candidate_id: "cand_concurrent_01".to_string(),
        title: "Concurrent Storage Test Candidate".to_string(),
        canonical_url: "https://example.com/test_cand".to_string(),
        published_at_utc: "2026-11-06T00:00:00Z".to_string(),
        ..Default::default()
    };
    assert!(storage.save_candidates(&[candidate]).is_ok());

    // Two concurrent workers competing to reserve the same candidate
    let s1 = storage.clone();
    let s2 = storage.clone();

    let h1 = thread::spawn(move || s1.reserve_handoff("cand_concurrent_01", 60, "worker_alpha"));
    let h2 = thread::spawn(move || s2.reserve_handoff("cand_concurrent_01", 60, "worker_beta"));

    let r1 = h1.join().unwrap();
    let r2 = h2.join().unwrap();

    // Exactly one worker must win the reservation, the other must be rejected
    let (winner_token, _winning_worker) = match (r1, r2) {
        (Ok(t), Err(_)) => (t, "worker_alpha"),
        (Err(_), Ok(t)) => (t, "worker_beta"),
        (Ok(_), Ok(_)) => panic!("RACE CONDITION: Both workers acquired the same reservation!"),
        (Err(e1), Err(e2)) => panic!("Both workers failed: e1={e1}, e2={e2}"),
    };

    // Attempt commit with wrong token must fail
    assert!(storage.commit_consumed("cand_concurrent_01", "invalid_token_999").is_err());

    // Commit with winning token must succeed atomically
    assert!(storage.commit_consumed("cand_concurrent_01", &winner_token).expect("Commit"));
    assert!(storage.is_consumed("cand_concurrent_01"));
    assert_eq!(storage.consumed_count(), 1);

    // Once published, releasing is prohibited
    assert!(storage.release_reservation("cand_concurrent_01", &winner_token).is_err());
}

// ===========================================================================
// 3. SQLite Legacy Migration: JSON Array Format Support (P0)
// ===========================================================================
#[test]
fn test_sqlite_automatic_json_migration_handles_legacy_array_format() {
    let tmp = make_temp_dir("sqlite_json_array_migration");
    let state_dir = tmp.join(".state");
    fs::create_dir_all(&state_dir).unwrap();

    // 1. config.json
    let config_json = r#"{"freshness_window_hours": 72.0, "auto_fallback": true}"#;
    fs::write(state_dir.join("config.json"), config_json).unwrap();

    // 2. candidates.json stored as a JSON ARRAY (as FileStorageAdapter::save_candidates writes)
    let cand1 = NormalizedCandidate {
        candidate_id: "cand_array_01".to_string(),
        title: "Array Candidate 01".to_string(),
        canonical_url: "https://example.com/array_01".to_string(),
        published_at_utc: "2026-11-06T00:00:00Z".to_string(),
        ..Default::default()
    };
    let cand2 = NormalizedCandidate {
        candidate_id: "cand_array_02".to_string(),
        title: "Array Candidate 02".to_string(),
        canonical_url: "https://example.com/array_02".to_string(),
        published_at_utc: "2026-11-06T00:00:00Z".to_string(),
        ..Default::default()
    };

    let array = vec![cand1, cand2];
    fs::write(state_dir.join("candidates.json"), serde_json::to_string(&array).unwrap()).unwrap();

    // 3. consumed.json stored as a JSON ARRAY of strings
    let consumed_json = r#"["cand_consumed_old_1", "cand_consumed_old_2"]"#;
    fs::write(state_dir.join("consumed.json"), consumed_json).unwrap();

    // Initialize SqliteStorageAdapter -> must automatically parse JSON array candidates and consumed
    let storage = SqliteStorageAdapter::new(&tmp).expect("Initialize and migrate JSON array");

    assert!(storage.is_consumed("cand_consumed_old_1"));
    assert!(storage.is_consumed("cand_consumed_old_2"));
    assert_eq!(storage.consumed_count(), 2);

    let c1 = storage.get_candidate("cand_array_01").expect("Candidate 1 migrated");
    assert_eq!(c1.title, "Array Candidate 01");
    let c2 = storage.get_candidate("cand_array_02").expect("Candidate 2 migrated");
    assert_eq!(c2.title, "Array Candidate 02");

    let cfg = storage.load_config();
    assert_eq!(cfg.freshness_window_hours, 72.0);
}

// ===========================================================================
// 4. Invariant Defense: Restart Cannot Downgrade PUBLISHED to RESERVED (P0)
// ===========================================================================
#[test]
fn test_sqlite_restart_preserves_published_status_and_migration_marker() {
    let tmp = make_temp_dir("sqlite_restart_published_invariant");
    let state_dir = tmp.join(".state");
    fs::create_dir_all(&state_dir).unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Initial state: reservations.json has cand_01 as RESERVED
    let mut res_map = std::collections::HashMap::new();
    res_map.insert(
        "cand_rev_test".to_string(),
        HandoffReservation {
            candidate_id: "cand_rev_test".to_string(),
            reservation_id: "res_old_1".to_string(),
            token_hash: compute_sha256("old_token"),
            worker_id: "worker_old".to_string(),
            status: ReservationStatus::Reserved,
            created_at: "2026-11-06T00:00:00Z".to_string(),
            expires_at: now + 3600,
        },
    );
    fs::write(
        state_dir.join("reservations.json"),
        serde_json::to_string(&res_map).unwrap(),
    )
    .unwrap();

    // 1. First initialization: migrations run once
    let storage1 = SqliteStorageAdapter::new(&tmp).expect("Init storage 1");
    let cand = NormalizedCandidate {
        candidate_id: "cand_rev_test".to_string(),
        title: "Test Candidate Invariant".to_string(),
        canonical_url: "https://example.com/inv".to_string(),
        published_at_utc: "2026-11-06T00:00:00Z".to_string(),
        ..Default::default()
    };
    storage1.save_candidates(&[cand]).unwrap();

    // Worker commits handoff as PUBLISHED
    let commit_res = storage1.commit_consumed("cand_rev_test", "old_token");
    assert!(commit_res.is_ok(), "Commit consumed as PUBLISHED");
    assert!(storage1.is_consumed("cand_rev_test"));

    // 2. Simulate process restart: create a new SqliteStorageAdapter instance against same folder
    // Legacy reservations.json STILL exists on disk with status RESERVED!
    let storage2 = SqliteStorageAdapter::new(&tmp).expect("Init storage 2 (restart)");

    // Must REMAIN published and consumed! Migration marker prevents overwrite.
    assert!(storage2.is_consumed("cand_rev_test"), "Must remain consumed after restart");

    // Token cannot release or steal the published reservation!
    let release_res = storage2.release_reservation("cand_rev_test", "old_token");
    assert!(release_res.is_err(), "PUBLISHED candidate cannot be released after restart!");
}

// ===========================================================================
// 5. Durability: PRAGMA synchronous = FULL (P1)
// ===========================================================================
#[test]
fn test_sqlite_durability_synchronous_full_pragma() {
    let tmp = make_temp_dir("sqlite_durability");
    let storage = SqliteStorageAdapter::new(&tmp).expect("Init SQLite");
    let conn = rusqlite::Connection::open(storage.db_path()).unwrap();
    let sync_val: i32 = conn
        .query_row("PRAGMA synchronous;", [], |r| r.get(0))
        .unwrap();
    // In SQLite, 2 corresponds to FULL synchronous mode for maximum durability
    assert_eq!(sync_val, 2, "SQLite synchronous mode must be FULL (2)");
}

// ===========================================================================
// 6. Strict License-Type Matching & Mutual Exclusion (P1)
// ===========================================================================
#[test]
fn test_strict_license_type_matching_cc_by_vs_sharealike() {
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let sha = compute_sha256_bytes(valid_png_bytes);

    // Case A: Metadata declares CC-BY, but license document requires ShareAlike (CC-BY-SA) -> MUST BE REJECTED!
    let visual_cc_by = VisualEvidence {
        asset_ref: "chart_cc.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/assets/chart_cc.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(sha.clone()),
        license_evidence_ref: Some("https://example.com/licenses/doc".to_string()),
        verified_rights: Some(true),
    };

    let cc_by_sa_doc = format!(
        "Title: chart_cc.png. Checksum: {sha}. Author: Copyright 2026 Acme Corp. Source: https://example.com/assets/chart_cc.png. \
         Licensed under Creative Commons Attribution-ShareAlike 4.0 International (CC BY-SA 4.0). Free for commercial use."
    );
    let res_sa = move |_r: &str| -> Option<String> { Some(cc_by_sa_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual_cc_by, Some(&res_sa)),
        "Metadata declaring cc-by but document imposing ShareAlike (CC-BY-SA) obligations must be rejected"
    );

    // Case B: Metadata declares CC-BY-SA, but license document is plain CC-BY without ShareAlike -> MUST BE REJECTED!
    let visual_cc_by_sa = VisualEvidence {
        license_status: Some("cc-by-sa".to_string()),
        ..visual_cc_by.clone()
    };
    let plain_cc_by_doc = format!(
        "Title: chart_cc.png. Checksum: {sha}. Author: Copyright 2026 Acme Corp. Source: https://example.com/assets/chart_cc.png. \
         Released under Creative Commons Attribution (CC-BY 4.0). Free for commercial use."
    );
    let res_plain_by = move |_r: &str| -> Option<String> { Some(plain_cc_by_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual_cc_by_sa, Some(&res_plain_by)),
        "Metadata declaring cc-by-sa but document lacking ShareAlike obligations must be rejected"
    );

    // Case C: Matching CC-BY declaration and Creative Commons grant document -> PASSES
    assert!(
        is_visual_rights_verified(&visual_cc_by, Some(&res_plain_by)),
        "Matching CC-BY declaration and plain Creative Commons grant document must pass"
    );

    // Case D: Matching CC-BY-SA declaration and Creative Commons ShareAlike grant document -> PASSES
    assert!(
        is_visual_rights_verified(&visual_cc_by_sa, Some(&res_sa)),
        "Matching CC-BY-SA declaration and Creative Commons ShareAlike grant document must pass"
    );
}

// ===========================================================================
// 7. Real Media Raster Image Decoding & Mock Asset Blocking (P1)
// ===========================================================================
#[test]
fn test_real_raster_image_decoding_and_mock_asset_rejection() {
    // Subtest A: Valid 1x1 RGBA PNG decodes successfully to real pixel buffer
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    assert!(decode_raster_image(valid_png_bytes), "Valid PNG must decode into real raster pixel buffer");
    assert!(is_visual_media_decodable(valid_png_bytes), "Valid PNG must be visual media decodable");

    // Subtest B: Synthetic payload with valid PNG header but corrupted raster stream data
    let corrupt_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x08\0\0\0\x08\x08\x02\0\0\0sp\x8b\xcf\0\0\0\x08IDATcorrupt\0\0\0\0IEND\xaeB`\x82";
    assert!(!decode_raster_image(corrupt_png), "Corrupt raster data must fail raster decoding");
    assert!(!is_visual_media_decodable(corrupt_png), "Corrupt raster data must fail visual media decodability");

    // Subtest C: Mock asset bytes MUST be rejected from real raster decoding and visual decodability
    let mock_bytes = b"MOCK_IMAGE_BYTES_PLACEHOLDER_001";
    assert!(!decode_raster_image(mock_bytes), "Mock bytes must NOT be accepted by real raster decoder");
    assert!(!is_visual_media_decodable(mock_bytes), "Mock bytes must NOT be accepted by visual media decodable");

    // Subtest D: Mock assets in evaluate_readiness_full cannot reach VIDEO_READY
    let mock_sha = compute_sha256_bytes(mock_bytes);
    let primary_text = "Verified primary benchmark documentation.";
    let primary_sha = compute_sha256(primary_text);
    let primary = PrimarySourceEvidence {
        url: "https://example.com/primary_doc".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        published_at: Some("2026-11-06T00:00:00Z".to_string()),
        snapshot_ref: Some(format!("{primary_sha}.txt")),
        snapshot_sha256: Some(primary_sha.clone()),
        ..Default::default()
    };
    let claim = ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "Verified benchmarks".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha.clone(),
            excerpt: "primary benchmark documentation".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "r1".to_string(),
            verified_at: "2026-11-06T00:00:00Z".to_string(),
            reviewer_id: "auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "audit".to_string(),
            audit_trail_ref: Some("sha_audit".to_string()),
            secondary_source_ref: None,
        }),
    };
    let visual = VisualEvidence {
        asset_ref: "mock_asset.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/mock.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("mock".to_string()),
        checksum_sha256: Some(mock_sha.clone()),
        license_evidence_ref: None,
        verified_rights: Some(true),
    };

    let asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == mock_sha { Some(mock_bytes.to_vec()) } else { None }
    };

    let readiness = evaluate_readiness_full(
        Some(&primary),
        Some(primary_text),
        &[claim],
        &[visual],
        ContentOrigin::Article,
        None,
        None,
        Some(&asset_res),
    );
    assert_ne!(readiness.status, ReadinessStatus::VideoReady, "Mock assets cannot reach VIDEO_READY");
}
