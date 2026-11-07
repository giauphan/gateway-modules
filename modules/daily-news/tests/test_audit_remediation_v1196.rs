//! Regression tests for Daily News v1.19.6 audit remediation:
//! - Security P0: Reject RFC 8032 test keys and test helper manipulation of production trust store
//! - Security P0: Require cryptographic signature content_sha256 to strictly match actual audit body
//! - Storage P1: Enforce PUBLISHED -> consumed state invariant on migration and is_consumed()
//! - Storage P1: Fail-closed on I/O error during mark_consumed
//! - Media P1: Reject corrupt/short raster images (no <= 64 bytes exception)
//! - Rights P1: Reject unverified / self-declared licensors

use daily_news::domain::{HandoffReservation, ReservationStatus, VisualEvidence};
use daily_news::research_readiness::media_decoder::is_visual_media_decodable;
use daily_news::research_readiness::verification::crypto::{
    global_trust_store, parse_attestation_envelope,
};
use daily_news::research_readiness::visual::is_visual_rights_verified;
use daily_news::storage::SqliteStorageAdapter;
use daily_news::ports::StoragePort;
use daily_news::url::compute_sha256;
use std::fs;

mod common;
use common::make_temp_dir;

// ---------------------------------------------------------------------------
// 1. Security P0: RFC 8032 public test key must NOT be seeded in production trust store
// ---------------------------------------------------------------------------
#[test]
fn test_rfc_8032_test_key_not_in_trust_store() {
    let store = global_trust_store();
    // RFC 8032 TEST 1 public key: d75a980182b10ab7d54bfed3c964073a...
    // Must NOT be present under mitre-notary-key-1 or any other alias
    let mitre_key = store.get_key("mitre-notary-key-1");
    assert!(
        mitre_key.is_none(),
        "Production trust store must NOT contain publicly known RFC 8032 test key for mitre-notary-key-1"
    );

    let certik_key = store.get_key("certik-notary-1");
    assert!(
        certik_key.is_none(),
        "Production trust store must NOT contain dummy sha256('foo') key for certik-notary-1"
    );
}

// ---------------------------------------------------------------------------
// 2. Security P0: Signature must strictly bind the actual audit body, not just envelope
// ---------------------------------------------------------------------------
#[test]
fn test_signature_must_strictly_match_actual_audit_body() {
    // If an attestation is appended to audit text A, but signed content_sha256 is for text B,
    // verify_parsed_attestation must REJECT even if envelope sha256 matches expected_hash!
    let actual_audit_body = "Real audit report findings: vulnerability CVE-2026-9999 verified.";
    let actual_body_sha = compute_sha256(actual_audit_body);

    let forged_audit_body = "Completely fake audit report claims zero vulnerabilities.";
    let forged_body_sha = compute_sha256(forged_audit_body);

    // Attestation signs forged_body_sha
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000);

    let test_secret = b"correct-test-signing-key-secret-1234";
    let store = daily_news::research_readiness::verification::crypto::TrustStore::new();
    store.register_key("test-key-id", daily_news::research_readiness::verification::crypto::TrustedKey::HmacSha256(test_secret.to_vec()));

    let signed_attestation_block = daily_news::research_readiness::verification::crypto::create_signed_gateway_attestation(
        "https://trailofbits.com/reports/audit.pdf",
        "https://trailofbits.com/reports/audit.pdf",
        &forged_body_sha, // Signing the FORGED body hash!
        now,
        "Trail of Bits",
        "test-key-id",
        test_secret,
    );

    let envelope_text = format!("{actual_audit_body}\n{signed_attestation_block}");
    let parsed = parse_attestation_envelope(&envelope_text).expect("Must parse envelope");
    let envelope_sha = compute_sha256(&envelope_text);

    // With the remediated security check, verify_parsed_attestation_with_body verifies that
    // attestation.content_sha256 matches actual_body_sha. Since attestation signed forged_body_sha,
    // this must be strictly rejected even if envelope_sha matches expected_hash!
    let verified = daily_news::research_readiness::verify_parsed_attestation_with_body(
        &parsed,
        "https://trailofbits.com/reports/audit.pdf",
        Some(&envelope_sha),
        Some(&envelope_sha),
        Some(&actual_body_sha),
        &store,
    );
    assert!(!verified, "Attestation signing a different content_sha256 must be rejected even if envelope hash matches");
}

// ---------------------------------------------------------------------------
// 3. Storage P1: SQLite migration of PUBLISHED reservation must populate consumed table
// ---------------------------------------------------------------------------
#[test]
fn test_sqlite_migration_published_reservation_ensures_consumed() {
    let tmp_dir = make_temp_dir("sqlite_mig_published");

    // Create legacy reservations.json inside .state/
    let state_dir = tmp_dir.join(".state");
    let _ = fs::create_dir_all(&state_dir);
    let published_cand_id = "cand_published_mig_test";
    let res = HandoffReservation {
        reservation_id: "res_pub_1".to_string(),
        candidate_id: published_cand_id.to_string(),
        token_hash: "thash".to_string(),
        worker_id: "worker-1".to_string(),
        created_at: "2026-11-06T00:00:00Z".to_string(),
        expires_at: 1899999999,
        status: ReservationStatus::Published,
    };
    let mut map = std::collections::HashMap::new();
    map.insert(published_cand_id.to_string(), res);
    fs::write(
        state_dir.join("reservations.json"),
        serde_json::to_string(&map).expect("Serialize reservations"),
    ).expect("Write reservations.json");

    // Note: consumed.json is intentionally EMPTY / absent!
    let adapter = SqliteStorageAdapter::new(&tmp_dir).expect("SQLite init must succeed");

    // Invariant: candidate was PUBLISHED, so is_consumed MUST be true!
    assert!(
        adapter.is_consumed(published_cand_id),
        "Candidate with PUBLISHED reservation must be reported as is_consumed=true after migration"
    );
}

// ---------------------------------------------------------------------------
// 4. Media P1: Corrupted or truncated short raster files must be rejected
// ---------------------------------------------------------------------------
#[test]
fn test_corrupt_short_jpeg_and_gif_rejected() {
    // 34-byte synthetic JPEG missing Huffman/quantization tables that cannot decode into pixels
    let corrupt_short_jpeg = vec![
        0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x05, 0x00, 0x01, 0x02,
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
        0x12, 0x34, 0xFF, 0xD9,
    ];
    assert!(
        !is_visual_media_decodable(&corrupt_short_jpeg),
        "Corrupted short JPEG without raster pixel decodability must be rejected"
    );

    // Corrupt short GIF
    let corrupt_short_gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff;\x00\x00\x00";
    assert!(
        !is_visual_media_decodable(corrupt_short_gif),
        "Corrupted short GIF without valid raster decoding must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 5. Rights P1: Self-declared or unverified licensor sources must be rejected
// ---------------------------------------------------------------------------
#[test]
fn test_unverified_self_declared_licensor_rejected() {
    let visual = VisualEvidence {
        asset_ref: "chart_1.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart_1.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()),
        license_evidence_ref: Some("https://example.com/license.txt".to_string()),
        verified_rights: Some(true),
    };

    // Self-certified / user-uploaded licensor text
    let self_certified_doc = "This asset is self-certified via user upload. Granted under Creative Commons Attribution for chart_1.png by author.";
    let resolver = Some(&|_ref_str: &str| -> Option<String> {
        Some(self_certified_doc.to_string())
    } as &dyn Fn(&str) -> Option<String>);

    assert!(
        !is_visual_rights_verified(&visual, resolver),
        "Self-certified or user upload licensor document must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 6. Security P0: Empty audit body cannot achieve verification
// ---------------------------------------------------------------------------
#[test]
fn test_empty_audit_body_rejected() {
    let empty_audit_body = "";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000);

    let test_secret = b"test-secret-for-empty-body-check";
    let store = daily_news::research_readiness::verification::crypto::TrustStore::new();
    store.register_key("test-key", daily_news::research_readiness::verification::crypto::TrustedKey::HmacSha256(test_secret.to_vec()));

    let empty_sha = compute_sha256(empty_audit_body);
    let attestation_block = daily_news::research_readiness::verification::crypto::create_signed_gateway_attestation(
        "https://certik.com/reports/audit.pdf",
        "https://certik.com/reports/audit.pdf",
        &empty_sha,
        now,
        "CertiK",
        "test-key",
        test_secret,
    );

    let envelope_text = format!("{empty_audit_body}\n{attestation_block}");
    let _parsed = parse_attestation_envelope(&envelope_text).expect("Parse envelope");

    let rec = daily_news::domain::VerificationRecord {
        record_id: "rec_empty".to_string(),
        verified_at: "2026-11-06T00:00:00Z".to_string(),
        reviewer_id: "certik-notary".to_string(),
        organization: "CertiK".to_string(),
        method: "cryptographic_attestation".to_string(),
        audit_trail_ref: Some(empty_sha),
        secondary_source_ref: Some("https://certik.com/reports/audit.pdf".to_string()),
    };

    let resolver = Some(&|_r: &str| -> Option<String> { Some(envelope_text.clone()) } as &dyn Fn(&str) -> Option<String>);
    assert!(
        !daily_news::research_readiness::is_audit_source_verified_with_resolver(&envelope_text, &rec, resolver),
        "Attestation with empty audit body must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 7. Media P1: Truncated VP8X WebP without raster chunk is rejected
// ---------------------------------------------------------------------------
#[test]
fn test_truncated_extended_webp_rejected() {
    // 30-byte WebP header with VP8X chunk, but without subsequent image chunk
    let mut truncated_vp8x = Vec::new();
    truncated_vp8x.extend_from_slice(b"RIFF");
    truncated_vp8x.extend_from_slice(&22u32.to_le_bytes()); // RIFF payload size
    truncated_vp8x.extend_from_slice(b"WEBP");
    truncated_vp8x.extend_from_slice(b"VP8X");
    truncated_vp8x.extend_from_slice(&10u32.to_le_bytes()); // VP8X chunk size = 10
    truncated_vp8x.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // flags
    truncated_vp8x.extend_from_slice(&[0x64, 0x00, 0x00]); // width-1 = 100
    truncated_vp8x.extend_from_slice(&[0x64, 0x00, 0x00]); // height-1 = 100

    assert!(
        !is_visual_media_decodable(&truncated_vp8x),
        "Truncated VP8X WebP missing image data chunk must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 8. Storage P1: is_consumed fails closed on query failure
// ---------------------------------------------------------------------------
#[test]
fn test_sqlite_is_consumed_fails_closed_on_error() {
    let tmp_dir = make_temp_dir("sqlite_fail_closed");
    let adapter = SqliteStorageAdapter::new(&tmp_dir).expect("SQLite init");

    // Intentionally drop the consumed table to simulate database corruption
    let db_path = tmp_dir.join(".state").join("daily_news.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open raw db");
    conn.execute("DROP TABLE consumed", []).expect("drop consumed table");

    // Under database query failure, is_consumed must FAIL CLOSED (return true) to prevent duplicate publishing
    assert!(
        adapter.is_consumed("any_candidate"),
        "is_consumed must fail closed (return true) when database query fails"
    );
}
