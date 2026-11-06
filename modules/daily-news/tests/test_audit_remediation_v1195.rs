//! Integration and regression test suite for Daily News v1.19.5 remediation:
//!
//! 1. P0: Cryptographic Signed Attestation (TrustStore, in-toto/gateway envelopes, Ed25519/HMAC-SHA256)
//! 2. P0: Transactional SQLite Storage (ACID transactions, WAL, concurrent worker locking, JSON migration)
//! 3. P1: Strict License-Type Matching & Structured TASL Rights Verification
//! 4. P1: Real Media Raster Image Decoding (Corrupted pixel payload rejection)

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use daily_news::domain::{
    ClaimEvidence, EvidenceRef, NormalizedCandidate, VerificationRecord, VisualEvidence,
};
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    create_signed_gateway_attestation, decode_raster_image, is_audit_source_verified_with_resolver,
    is_visual_rights_verified, AuditProvenanceStatus,
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

    // Subtest A: Forged signature with fake key ID must FAIL closed
    let forged_key_attestation = format!(
        "{audit_text}\n-----BEGIN GATEWAY FETCH ATTESTATION-----\n\
         Version: 1\n\
         URL: {sec_url}\n\
         Final-URL: {sec_url}\n\
         Content-SHA256: {audit_sha}\n\
         Fetched-At: 1793618000\n\
         Issuer: CertiK\n\
         Key-ID: unknown-rogue-key-id\n\
         TLS-Verified: true\n\
         Signature: badc0ffee0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n\
         -----END GATEWAY FETCH ATTESTATION-----"
    );
    assert!(
        !is_audit_source_verified_with_resolver(&forged_key_attestation, &rec, Some(&resolver)),
        "Unknown key ID in attestation must fail closed"
    );

    // Subtest B: Tampered content hash in attestation must FAIL closed
    let tampered_hash = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    let tampered_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        tampered_hash,
        1793618000,
        "CertiK",
        "gateway-internal-hmac-1",
        b"gateway-production-attestation-secret-2026",
    );
    let tampered_doc = format!("{audit_text}\n{tampered_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&tampered_doc, &rec, Some(&resolver)),
        "Tampered content hash in attestation must fail closed"
    );

    // Subtest C: Malicious redirect to localhost or private IP in Final-URL must FAIL closed
    let ssrf_redirect_attestation = create_signed_gateway_attestation(
        sec_url,
        "http://127.0.0.1:8080/internal_secrets",
        &audit_sha,
        1793618000,
        "CertiK",
        "gateway-internal-hmac-1",
        b"gateway-production-attestation-secret-2026",
    );
    let ssrf_doc = format!("{audit_text}\n{ssrf_redirect_attestation}");
    assert!(
        !is_audit_source_verified_with_resolver(&ssrf_doc, &rec, Some(&resolver)),
        "SSRF redirect in attestation must fail closed"
    );

    // Subtest D: Authentic cryptographically signed attestation passes and achieves IssuerAuthenticated
    let authentic_attestation = create_signed_gateway_attestation(
        sec_url,
        sec_url,
        &audit_sha,
        1793618000,
        "CertiK",
        "gateway-internal-hmac-1",
        b"gateway-production-attestation-secret-2026",
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
        "Authentic signed attestation must pass source verification"
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

#[test]
fn test_sqlite_automatic_json_migration() {
    let tmp = make_temp_dir("sqlite_json_migration");
    let state_dir = tmp.join(".state");
    fs::create_dir_all(&state_dir).unwrap();

    // Populate legacy JSON files
    let config_json = r#"{"freshness_window_hours": 72.0, "auto_fallback": true}"#;
    fs::write(state_dir.join("config.json"), config_json).unwrap();

    let mut cand = NormalizedCandidate::default();
    cand.candidate_id = "cand_mig_01".to_string();
    cand.title = "Migrated Candidate 01".to_string();
    cand.canonical_url = "https://example.com/mig_01".to_string();
    cand.published_at_utc = "2026-11-06T00:00:00Z".to_string();
    let mut map = std::collections::HashMap::new();
    map.insert("cand_mig_01".to_string(), cand);
    fs::write(state_dir.join("candidates.json"), serde_json::to_string(&map).unwrap()).unwrap();

    let consumed_json = r#"["cand_consumed_old"]"#;
    fs::write(state_dir.join("consumed.json"), consumed_json).unwrap();

    // Initialize SqliteStorageAdapter -> must automatically migrate all records
    let storage = SqliteStorageAdapter::new(&tmp).expect("Initialize and migrate");

    assert!(storage.is_consumed("cand_consumed_old"));
    assert_eq!(storage.consumed_count(), 1);

    let cand = storage.get_candidate("cand_mig_01").expect("Migrated candidate found");
    assert_eq!(cand.title, "Migrated Candidate 01");

    let cfg = storage.load_config();
    assert_eq!(cfg.freshness_window_hours, 72.0);
}

// ===========================================================================
// 3. Strict License-Type Matching & Structured TASL (P1)
// ===========================================================================
#[test]
fn test_strict_license_type_matching_rejects_mismatch() {
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let sha = compute_sha256_bytes(valid_png_bytes);

    let visual_cc = VisualEvidence {
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

    // Document contains MIT License text, but metadata declared CC-BY -> MISMATCH MUST BE REJECTED!
    let mit_license_text = format!(
        "Title: chart_cc.png. Checksum: {sha}. Author: Copyright 2026 Acme Corp. Source: https://example.com/assets/chart_cc.png. \
         Permission is hereby granted, free of charge, under the MIT License. Free for commercial use."
    );
    let resolver_mit = move |_r: &str| -> Option<String> { Some(mit_license_text.clone()) };
    assert!(
        !is_visual_rights_verified(&visual_cc, Some(&resolver_mit)),
        "Metadata declaring cc-by but document offering MIT License MUST be rejected as license type mismatch"
    );

    // Document contains authentic Creative Commons Attribution text -> PASSES
    let cc_license_text = format!(
        "Title: chart_cc.png. Checksum: {sha}. Author: Copyright 2026 Acme Corp. Source: https://example.com/assets/chart_cc.png. \
         Released under Creative Commons Attribution (CC-BY 4.0). Free for commercial use."
    );
    let resolver_cc = move |_r: &str| -> Option<String> { Some(cc_license_text.clone()) };
    assert!(
        is_visual_rights_verified(&visual_cc, Some(&resolver_cc)),
        "Matching CC-BY declaration and Creative Commons grant document must pass"
    );
}

// ===========================================================================
// 4. Real Media Raster Image Decoding (P1)
// ===========================================================================
#[test]
fn test_real_raster_image_decoding_integrity() {
    // Valid 1x1 RGBA PNG decodes successfully to real pixel buffer
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    assert!(decode_raster_image(valid_png_bytes), "Valid PNG must decode into real raster pixel buffer");

    // Synthetic payload with valid PNG header but corrupted raster stream data
    let corrupt_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x08\0\0\0\x08\x08\x02\0\0\0sp\x8b\xcf\0\0\0\x08IDATcorrupt\0\0\0\0IEND\xaeB`\x82";
    assert!(!decode_raster_image(corrupt_png), "Corrupt raster data must fail raster decoding");
}
