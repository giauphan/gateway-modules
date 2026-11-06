//! Regression and integration test suite for Daily News v1.19.4:
//! 1. `test_audit_provenance_rejects_unauthenticated_mock_resolver_for_video_ready`:
//!    Unauthenticated mock resolver yields `ResolvedDocument`, fails closed for `VIDEO_READY` (yields `NeedsVerification`).
//!    Authentic cryptographic signature / trusted attestation yields `IssuerAuthenticated` and `VideoReady`.
//! 2. `test_license_tasl_and_asset_sha256_binding_required`:
//!    Visual asset requires CC TASL (Title, Author, Source, License) and cryptographic binding to asset sha256.
//!    Floating or generic unlinked license claims fail closed.
//! 3. `test_transactional_storage_wal_journal_reconciliation`:
//!    FileStorageAdapter commit journal handles simulated crashes between writing reservations and consumed;
//!    startup reconciliation recovers committed state seamlessly.
//! 4. `test_storage_persistence_error_propagation`:
//!    Persistence failure in storage adapter and service discovery fails closed, leaving cache uncorrupted.
//! 5. `test_real_media_stream_decoders`:
//!    Rejects corrupted/truncated JPEG (missing scan data or invalid byte stuffing),
//!    corrupted GIF (bad LZW min code size or broken dictionary codes),
//!    and corrupted PNG (bad Adam7 interlaced scanline filter).

use daily_news::date::current_utc_timestamp;
use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, HandoffReservation, ModuleConfig,
    NormalizedCandidate, PrimarySourceEvidence, RawNewsItem, ReadinessStatus,
    ReservationStatus, VerificationRecord, VisualEvidence,
};
use daily_news::ports::{FeedFetcherPort, StoragePort};
use daily_news::research_readiness::{
    are_visuals_provenance_verified_full, check_audit_provenance_level_with_resolver,
    evaluate_readiness_full, is_audit_source_verified_with_resolver,
    is_visual_media_decodable, is_visual_rights_verified, verify_visual_asset_record,
    AuditProvenanceStatus,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::FileStorageAdapter;
use daily_news::url::{compute_sha256, compute_sha256_bytes};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_v1194_{tag}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

struct MockFeedFetcher {
    items: Vec<RawNewsItem>,
}

impl FeedFetcherPort for MockFeedFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(self.items.clone())
    }
    fn fetch_sources(&self, _sources: &[String], _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(self.items.clone())
    }
}

fn make_png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(chunk_type);
    hasher.update(data);
    chunk.extend_from_slice(&hasher.finalize().to_be_bytes());
    chunk
}

fn make_adam7_png(corrupt_filter_byte: Option<u8>) -> Vec<u8> {
    let mut png = Vec::from(b"\x89PNG\r\n\x1a\n");
    let ihdr_data = [0, 0, 0, 2, 0, 0, 0, 2, 8, 0, 0, 0, 1]; // 2x2, 8bpp grayscale, interlace=1
    png.extend_from_slice(&make_png_chunk(b"IHDR", &ihdr_data));
    let raw = vec![corrupt_filter_byte.unwrap_or(0), 10, 0, 20, 0, 30, 40];
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    png.extend_from_slice(&make_png_chunk(b"IDAT", &compressed));
    png.extend_from_slice(&make_png_chunk(b"IEND", &[]));
    png
}

fn make_valid_jpeg() -> Vec<u8> {
    vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xDB, 0x00, 0x05, 0x00, 0x01, 0x02, // DQT
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00, // SOF0: 1x1, 3 comp
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, // SOS
        0x12, 0x34, 0xFF, 0x00, 0x56, // Scan entropy bytes with valid byte stuffing
        0xFF, 0xD9, // EOI
    ]
}

fn make_valid_gif() -> Vec<u8> {
    vec![
        b'G', b'I', b'F', b'8', b'9', b'a',
        0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, // Screen: 1x1, GCT flag
        0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, // GCT (2 colors)
        0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // Image descriptor
        0x02, 0x02, 0x44, 0x01, 0x00, // LZW data: min size 2, clear code 4, eoi 5
        0x3B, // Trailer
    ]
}

// ===========================================================================
// 1. Audit Provenance: Rejects Unauthenticated Mock Resolver for VIDEO_READY
// ===========================================================================
#[test]
fn test_audit_provenance_rejects_unauthenticated_mock_resolver_for_video_ready() {
    let audit_text = "Independent benchmark audit performed by CertiK confirms 4x throughput improvement with formal verification across clusters. Audit findings confirmed and verified by CertiK.";
    let audit_sha = compute_sha256(audit_text);

    let rec = VerificationRecord {
        record_id: "certik_audit_2026_99".to_string(),
        verified_at: "2026-11-06T00:00:00Z".to_string(),
        reviewer_id: "certik_lead_auditor".to_string(),
        organization: "CertiK".to_string(),
        method: "formal_verification".to_string(),
        audit_trail_ref: Some(audit_sha.clone()),
        secondary_source_ref: Some("https://certik.com/reports/certik_audit_2026_99".to_string()),
    };

    let primary_text = "Verified primary benchmark documentation reports 4x throughput across clusters.";
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
        claim_id: "c_throughput_01".to_string(),
        statement: "4x throughput across clusters".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha.clone(),
            excerpt: "4x throughput across clusters".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(rec.clone()),
    };

    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let asset_sha = compute_sha256_bytes(valid_png_bytes);

    let visual = VisualEvidence {
        asset_ref: "perf_chart.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/assets/perf_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(asset_sha.clone()),
        license_evidence_ref: Some("https://example.com/licenses/perf_chart".to_string()),
        verified_rights: Some(true),
    };

    let audit_sha_for_res = audit_sha.clone();
    let audit_resolver = move |h: &str| -> Option<String> {
        if h == audit_sha_for_res { Some(audit_text.to_string()) } else { None }
    };
    let asset_resolver = move |sha: &str| -> Option<Vec<u8>> {
        if sha == asset_sha { Some(valid_png_bytes.to_vec()) } else { None }
    };

    // Case 1: Unauthenticated mock resolver returning text without authentic cryptographic
    // signature or trusted attestation. Only achieves ResolvedDocument, NOT IssuerAuthenticated!
    let mock_text = "CertiK mentions certik_audit_2026_99 in an informal blog post discussion with unverified notes.";
    let mock_resolver = move |_r: &str| -> Option<String> { Some(mock_text.to_string()) };

    let status_mock = check_audit_provenance_level_with_resolver(
        audit_text,
        &rec,
        &claim,
        Some(&mock_resolver),
    );
    assert_eq!(
        status_mock,
        AuditProvenanceStatus::ResolvedDocument,
        "Mock unauthenticated resolver must only achieve ResolvedDocument"
    );
    assert_ne!(
        status_mock,
        AuditProvenanceStatus::IssuerAuthenticated,
        "Mock unauthenticated resolver must NOT achieve IssuerAuthenticated"
    );
    assert!(
        !is_audit_source_verified_with_resolver(audit_text, &rec, Some(&mock_resolver)),
        "Mock resolver must fail source verification"
    );

    // VIDEO_READY gate MUST fail closed (yields NeedsVerification)
    let readiness_mock = evaluate_readiness_full(
        Some(&primary),
        Some(primary_text),
        &[claim.clone()],
        &[visual.clone()],
        ContentOrigin::Article,
        Some(&mock_resolver),
        Some(&audit_resolver),
        Some(&asset_resolver),
    );
    assert_eq!(
        readiness_mock.status,
        ReadinessStatus::NeedsVerification,
        "Readiness MUST fail closed to NeedsVerification when audit source lacks authentic attestation"
    );
    assert!(readiness_mock.missing_fields.contains(&"source_verified".to_string()));

    // Case 2: Authentic published audit report with formal verification structure
    // AND authentic gateway fetch attestation with cryptographic signature.
    let authentic_audit_doc = "Official CertiK Benchmark Audit Report 2026-99. Security assessment and formal verification for certik_audit_2026_99 passed. Audit findings confirmed by CertiK.";
    let license_doc = "Title: perf_chart.png. Author: Copyright 2026 Example Corp. Source: https://example.com/assets/perf_chart.png. Released under Creative Commons Attribution (CC-BY). Free for commercial use.";
    let authentic_resolver = move |r: &str| -> Option<String> {
        if r.contains("certik") {
            Some(authentic_audit_doc.to_string())
        } else if r.contains("perf_chart") || r.contains("license") {
            Some(license_doc.to_string())
        } else {
            Some(authentic_audit_doc.to_string())
        }
    };

    // Case 2a: Fake / forged attestation signature: MUST FAIL CLOSED!
    let fake_attested_audit_text = format!(
        "{audit_text}\n-----BEGIN GATEWAY FETCH ATTESTATION-----\nVersion: 1\nURL: https://certik.com/reports/certik_audit_2026_99\nContent-SHA256: {audit_sha}\nFetched-At: 1793618000\nIssuer: CertiK\nKey-ID: gateway-internal-hmac-1\nTLS-Verified: true\nSignature: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n-----END GATEWAY FETCH ATTESTATION-----"
    );
    let fake_attested_sha = compute_sha256(&fake_attested_audit_text);
    let rec_fake = VerificationRecord {
        audit_trail_ref: Some(fake_attested_sha),
        ..rec.clone()
    };
    let claim_fake = ClaimEvidence {
        verification_record: Some(rec_fake.clone()),
        ..claim.clone()
    };
    assert!(
        !is_audit_source_verified_with_resolver(&fake_attested_audit_text, &rec_fake, Some(&authentic_resolver)),
        "Fake attestation signature MUST fail cryptographic source verification"
    );
    assert_ne!(
        check_audit_provenance_level_with_resolver(
            &fake_attested_audit_text,
            &rec_fake,
            &claim_fake,
            Some(&authentic_resolver),
        ),
        AuditProvenanceStatus::IssuerAuthenticated,
        "Fake attestation signature CANNOT yield IssuerAuthenticated"
    );

    // Case 2b: Cryptographically signed authentic gateway fetch attestation
    let attestation_block = daily_news::research_readiness::create_signed_gateway_attestation(
        "https://certik.com/reports/certik_audit_2026_99",
        "https://certik.com/reports/certik_audit_2026_99",
        &audit_sha,
        1793618000,
        "CertiK",
        "gateway-internal-hmac-1",
        b"gateway-production-attestation-secret-2026",
    );
    let attested_audit_text = format!("{audit_text}\n{attestation_block}");
    let attested_sha = compute_sha256(&attested_audit_text);
    let rec_attested = VerificationRecord {
        audit_trail_ref: Some(attested_sha.clone()),
        ..rec.clone()
    };
    let claim_attested = ClaimEvidence {
        verification_record: Some(rec_attested.clone()),
        ..claim.clone()
    };

    let status_auth = check_audit_provenance_level_with_resolver(
        &attested_audit_text,
        &rec_attested,
        &claim_attested,
        Some(&authentic_resolver),
    );
    assert_eq!(
        status_auth,
        AuditProvenanceStatus::IssuerAuthenticated,
        "Authentic attestation must yield IssuerAuthenticated"
    );
    assert!(
        is_audit_source_verified_with_resolver(&attested_audit_text, &rec_attested, Some(&authentic_resolver)),
        "Authentic report must pass source verification"
    );

    let attested_audit_resolver = move |h: &str| -> Option<String> {
        if h == attested_sha { Some(attested_audit_text.clone()) } else { None }
    };

    let readiness_auth = evaluate_readiness_full(
        Some(&primary),
        Some(primary_text),
        &[claim_attested],
        &[visual],
        ContentOrigin::Article,
        Some(&authentic_resolver),
        Some(&attested_audit_resolver),
        Some(&asset_resolver),
    );
    assert_eq!(
        readiness_auth.status,
        ReadinessStatus::VideoReady,
        "Authentic attestation must satisfy VIDEO_READY"
    );
}

// ===========================================================================
// 2. License CC TASL & Cryptographic Asset SHA-256 Binding Required
// ===========================================================================
#[test]
fn test_license_tasl_and_asset_sha256_binding_required() {
    let valid_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let asset_sha = compute_sha256_bytes(valid_png_bytes);

    let valid_tasl_doc = format!(
        "Title: architecture_diagram.png. Author: Copyright 2026 Example Research Foundation. Source: https://example.com/assets/architecture_diagram.png. License: Released under Creative Commons Attribution (CC-BY) 4.0. Permitted for commercial use. Cryptographic Asset SHA-256: {asset_sha}."
    );
    let tasl_sha = compute_sha256(&valid_tasl_doc);

    let visual = VisualEvidence {
        asset_ref: "architecture_diagram.png".to_string(),
        kind: "diagram".to_string(),
        source_url: "https://example.com/assets/architecture_diagram.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(asset_sha.clone()),
        license_evidence_ref: Some(tasl_sha),
        verified_rights: Some(true),
    };

    // Case 2A: Floating license claim (no title/asset_ref, no sha256 binding, no source url/filename)
    let floating_doc = "This creative work is released under Creative Commons Attribution (CC-BY). Copyright 2026 Example Corp. Free for commercial use.";
    let res_floating = move |_r: &str| -> Option<String> { Some(floating_doc.to_string()) };
    assert!(
        !is_visual_rights_verified(&visual, Some(&res_floating)),
        "Floating license claim without asset linkage must fail closed"
    );

    // Case 2B: Generic license claim referencing a foreign/mismatched sha256 checksum
    let foreign_sha = compute_sha256("foreign_unrelated_asset_bytes");
    let unlinked_doc = format!(
        "Creative Commons Attribution (CC-BY). Copyright 2026 Example Corp. Bound to asset sha256: {foreign_sha}. Permitted for commercial use."
    );
    let res_unlinked = move |_r: &str| -> Option<String> { Some(unlinked_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual, Some(&res_unlinked)),
        "Generic claim bound to mismatched sha256 must fail closed"
    );

    // Case 2C: Claim for unrelated asset title
    let wrong_title_doc = "Title: other_chart.png. Author: Copyright 2026 Example Corp. Source: https://example.com/other.png. Released under CC-BY. Permitted for commercial use.".to_string();
    let res_wrong_title = move |_r: &str| -> Option<String> { Some(wrong_title_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual, Some(&res_wrong_title)),
        "Claim for a different asset title must fail closed"
    );

    // Case 2D: Missing Author / Licensor attribution in TASL
    let no_author_doc = "Title: architecture_diagram.png. Source: https://example.com/assets/architecture_diagram.png. Released under Creative Commons Attribution (CC-BY). Free for commercial use. Unattributed creator.".to_string();
    let res_no_author = move |_r: &str| -> Option<String> { Some(no_author_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual, Some(&res_no_author)),
        "TASL lacking author attribution must fail closed"
    );

    // Case 2E: Restrictive / Non-commercial clause in TASL document
    let restrictive_doc = "Title: architecture_diagram.png. Author: Copyright 2026 Example Corp. Source: https://example.com/assets/architecture_diagram.png. License required before commercial use.".to_string();
    let res_restrictive = move |_r: &str| -> Option<String> { Some(restrictive_doc.clone()) };
    assert!(
        !is_visual_rights_verified(&visual, Some(&res_restrictive)),
        "Restrictive clause in TASL document must fail closed"
    );

    // Case 2F: Valid CC TASL with cryptographic binding to asset sha256
    let tasl_doc_clone = valid_tasl_doc.clone();
    let res_valid_tasl = move |_r: &str| -> Option<String> { Some(tasl_doc_clone.clone()) };
    assert!(
        is_visual_rights_verified(&visual, Some(&res_valid_tasl)),
        "Valid CC TASL bound to asset sha256 and licensor entity must succeed"
    );

    // Case 2G: Full provenance verification with asset bytes and record verification
    let asset_sha_clone = asset_sha.clone();
    let asset_resolver = move |sha: &str| -> Option<Vec<u8>> {
        if sha == asset_sha_clone { Some(valid_png_bytes.to_vec()) } else { None }
    };
    assert!(are_visuals_provenance_verified_full(
        &[visual.clone()],
        Some(&asset_resolver),
        Some(&res_valid_tasl)
    ));
    assert!(!are_visuals_provenance_verified_full(
        &[visual.clone()],
        Some(&asset_resolver),
        Some(&res_floating)
    ));

    let record = verify_visual_asset_record(&visual, Some(valid_png_bytes), Some(&valid_tasl_doc))
        .expect("Record verification must succeed with valid TASL and sha256 bytes");
    assert!(record.bytes_verified);
    assert!(record.license_verified);
    assert_eq!(record.checksum_sha256, asset_sha);
}

// ===========================================================================
// 3. Transactional Storage WAL / Journal Reconciliation Across Crashes
// ===========================================================================
#[test]
fn test_transactional_storage_wal_journal_reconciliation() {
    let tmp = make_temp_dir("wal_journal_reconcile");
    let state_dir = tmp.join(".state");
    let reservations_file = state_dir.join("reservations.json");
    let consumed_file = state_dir.join("consumed.json");

    // 1. Initial storage lifecycle
    {
        let storage = FileStorageAdapter::new(&tmp);
        let cand_alpha = NormalizedCandidate {
            candidate_id: "cand_wal_alpha".to_string(),
            title: "WAL Consistency Alpha".to_string(),
            published_at_utc: current_utc_timestamp(),
            ..Default::default()
        };
        let cand_beta = NormalizedCandidate {
            candidate_id: "cand_wal_beta".to_string(),
            title: "WAL Consistency Beta".to_string(),
            published_at_utc: current_utc_timestamp(),
            ..Default::default()
        };
        storage.upsert_candidates(&[cand_alpha, cand_beta]);

        let token_alpha = storage.reserve_handoff("cand_wal_alpha", 3600, "worker_wal_1").unwrap();
        let token_beta = storage.reserve_handoff("cand_wal_beta", 3600, "worker_wal_2").unwrap();

        assert!(storage.is_consumed("cand_wal_alpha"));
        assert!(storage.is_consumed("cand_wal_beta"));

        storage.commit_consumed("cand_wal_alpha", &token_alpha).unwrap();
        storage.commit_consumed("cand_wal_beta", &token_beta).unwrap();
    }

    // 2. Simulate crash between writing reservations and consumed:
    // reservations.json is committed with Published status, but consumed.json was lost / corrupted.
    assert!(reservations_file.is_file());
    let _ = fs::remove_file(&consumed_file);
    assert!(!consumed_file.exists(), "Simulated crash: consumed.json lost");

    let res_content = fs::read_to_string(&reservations_file).unwrap();
    let reservations_map: HashMap<String, HandoffReservation> = serde_json::from_str(&res_content).unwrap();
    assert_eq!(reservations_map["cand_wal_alpha"].status, ReservationStatus::Published);
    assert_eq!(reservations_map["cand_wal_beta"].status, ReservationStatus::Published);

    // 3. Restart storage adapter: startup reconciliation recovers committed state from journal
    {
        let recovered_storage = FileStorageAdapter::new(&tmp);
        assert!(recovered_storage.is_consumed("cand_wal_alpha"));
        assert!(recovered_storage.is_consumed("cand_wal_beta"));
        assert_eq!(recovered_storage.consumed_count(), 2);

        // Invariance: Cannot re-reserve or release published candidates
        assert!(recovered_storage.reserve_handoff("cand_wal_alpha", 3600, "worker_3").is_err());
        assert!(recovered_storage.release_reservation("cand_wal_alpha", "tok").is_err());

        // 4. Durability: Mark a third candidate consumed and verify flush persists all
        let added = recovered_storage.mark_consumed(&["cand_wal_gamma".to_string()]).unwrap();
        assert_eq!(added, 1);
        assert!(consumed_file.is_file());
    }

    // 5. Simulate mid-flight crash at Point of Commit (commit_tx.journal written, process killed before reservations.json update)
    let journal_path = state_dir.join("commit_tx.journal");
    let journal_res = HandoffReservation {
        reservation_id: "res_journal_wal_01".to_string(),
        candidate_id: "cand_wal_delta".to_string(),
        token_hash: "dummy_token_hash".to_string(),
        worker_id: "worker_wal_journal".to_string(),
        created_at: current_utc_timestamp(),
        expires_at: 1999999999,
        status: ReservationStatus::Published,
    };
    let journal_entry = serde_json::json!({
        "candidate_id": "cand_wal_delta",
        "published_reservation": journal_res,
        "timestamp": 1793618000,
    });
    fs::write(&journal_path, serde_json::to_string_pretty(&journal_entry).unwrap()).unwrap();
    assert!(journal_path.is_file());

    // 6. Restart recovers candidate from commit_tx.journal and cleans up the journal file
    {
        let recheck_storage = FileStorageAdapter::new(&tmp);
        assert!(recheck_storage.is_consumed("cand_wal_alpha"));
        assert!(recheck_storage.is_consumed("cand_wal_beta"));
        assert!(recheck_storage.is_consumed("cand_wal_gamma"));
        assert!(recheck_storage.is_consumed("cand_wal_delta"));
        assert!(!journal_path.exists(), "commit_tx.journal must be cleared after reconciliation");
        assert_eq!(recheck_storage.consumed_count(), 4);
    }
}

// ===========================================================================
// 4. Storage Persistence Error Propagation (Fails Closed)
// ===========================================================================
#[test]
fn test_storage_persistence_error_propagation() {
    let tmp = make_temp_dir("storage_err_prop");
    let state_dir = tmp.join(".state");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));

    // 1. Initial valid candidate persists successfully
    let initial_candidate = NormalizedCandidate {
        candidate_id: "cand_pers_valid_01".to_string(),
        title: "Initial Valid Candidate".to_string(),
        canonical_url: "https://example.com/initial".to_string(),
        published_at_utc: current_utc_timestamp(),
        ..Default::default()
    };
    assert!(storage.save_candidates(&[initial_candidate]).is_ok());
    assert_eq!(storage.get_cached_candidates().len(), 1);

    // 2. Induce disk write failure: convert candidates.json into a non-writable directory
    let candidates_path = state_dir.join("candidates.json");
    let _ = fs::remove_file(&candidates_path);
    fs::create_dir_all(&candidates_path).expect("Create blocking directory");

    // 3. Attempt direct storage save: MUST propagate error and NOT silently succeed
    let failing_candidate = NormalizedCandidate {
        candidate_id: "cand_pers_failing_02".to_string(),
        title: "Failing Write Candidate".to_string(),
        canonical_url: "https://example.com/failing".to_string(),
        published_at_utc: current_utc_timestamp(),
        ..Default::default()
    };
    let save_res = storage.save_candidates(&[failing_candidate]);
    assert!(save_res.is_err(), "save_candidates MUST fail closed on disk write failure");

    // RAM cache MUST NOT be corrupted by unpersisted candidate
    let cached_after_direct_fail = storage.get_cached_candidates();
    assert_eq!(cached_after_direct_fail.len(), 1);
    assert_eq!(cached_after_direct_fail[0].candidate_id, "cand_pers_valid_01");

    // 4. Verify try_get_candidates and service discover error propagation
    let now_ts = current_utc_timestamp();
    let raw_item = RawNewsItem::new(
        "raw_discover_01",
        "https://example.com/raw_01",
        "Discovery Test Raw Item",
        &now_ts,
    );
    let fetcher = Arc::new(MockFeedFetcher { items: vec![raw_item.clone()] });
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

    // try_get_candidates MUST fail closed on disk write failure
    let try_res = service.try_get_candidates(10);
    assert!(try_res.is_err(), "service.try_get_candidates MUST return Err on persistence failure");

    // get_candidates fails closed to empty vector on persistence failure
    let candidates_fallback = service.get_candidates(10);
    assert!(candidates_fallback.is_empty(), "service.get_candidates MUST fail closed to empty on persistence error");

    // service.discover MUST propagate persistence error
    let discover_res = service.discover(10, Some(vec![raw_item]), None, None, None, None);
    assert!(discover_res.is_err(), "service.discover MUST propagate persistence error");
    let (err_tier, err_msg) = discover_res.unwrap_err();
    assert_eq!(err_tier, "PERSISTENCE_ERROR");
    assert!(err_msg.contains("Failed to persist candidates"));

    assert_eq!(storage.get_cached_candidates().len(), 1);
    let _ = fs::remove_dir_all(&candidates_path);

    // 5. Verify config persistence error propagation when config file is blocked
    let config_path = state_dir.join("config.json");
    let _ = fs::remove_file(&config_path);
    fs::create_dir_all(&config_path).expect("Create blocking dir for config");

    let cfg = ModuleConfig::default();
    assert!(storage.save_config(&cfg).is_err(), "save_config MUST fail closed when disk write fails");
    let _ = fs::remove_dir_all(&config_path);
}

// ===========================================================================
// 5. Real Media Stream Decoders (Corrupted Media Stream Rejections)
// ===========================================================================
#[test]
fn test_real_media_stream_decoders() {
    // A. PNG: Adam7 Interlaced Scanline Filter Validation
    let valid_adam7 = make_adam7_png(None);
    assert!(is_visual_media_decodable(&valid_adam7), "Valid Adam7 interlaced PNG must be accepted");

    let corrupt_adam7_filter_5 = make_adam7_png(Some(5));
    assert!(!is_visual_media_decodable(&corrupt_adam7_filter_5), "Adam7 PNG with filter byte 5 must be rejected");

    let corrupt_adam7_filter_ff = make_adam7_png(Some(0xFF));
    assert!(!is_visual_media_decodable(&corrupt_adam7_filter_ff), "Adam7 PNG with filter byte 0xFF must be rejected");

    // B. JPEG: Missing Scan Data, Invalid Byte Stuffing, and Truncation
    let valid_jpeg = make_valid_jpeg();
    assert!(is_visual_media_decodable(&valid_jpeg), "Valid JPEG must be accepted");

    // Missing scan data (SOS immediately followed by EOI with 0 entropy bytes)
    let jpeg_missing_scan = vec![
        0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x05, 0x00, 0x01, 0x02,
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0xFF, 0xD9,
    ];
    assert!(!is_visual_media_decodable(&jpeg_missing_scan), "JPEG with 0 scan entropy bytes must be rejected");

    // Invalid byte stuffing in SOS scan data (0xFF followed by 0x42 instead of 0x00/restart/EOI)
    let jpeg_invalid_byte_stuffing = vec![
        0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x05, 0x00, 0x01, 0x02,
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
        0x12, 0xFF, 0x42, 0x34, 0xFF, 0xD9,
    ];
    assert!(!is_visual_media_decodable(&jpeg_invalid_byte_stuffing), "JPEG with invalid byte stuffing (0xFF 0x42) must be rejected");

    let mut jpeg_truncated = valid_jpeg.clone();
    jpeg_truncated.truncate(valid_jpeg.len() - 3);
    assert!(!is_visual_media_decodable(&jpeg_truncated), "Truncated JPEG missing terminating EOI must be rejected");

    // C. GIF: Bad LZW Min Code Size and Broken Dictionary Codes
    let valid_gif = make_valid_gif();
    assert!(is_visual_media_decodable(&valid_gif), "Valid GIF89a stream must be accepted");

    let mut gif_bad_min_code_1 = valid_gif.clone();
    gif_bad_min_code_1[29] = 0x01;
    assert!(!is_visual_media_decodable(&gif_bad_min_code_1), "GIF with min code size 1 must be rejected");

    let mut gif_bad_min_code_0 = valid_gif.clone();
    gif_bad_min_code_0[29] = 0x00;
    assert!(!is_visual_media_decodable(&gif_bad_min_code_0), "GIF with min code size 0 must be rejected");

    let mut gif_bad_min_code_12 = valid_gif.clone();
    gif_bad_min_code_12[29] = 0x0C;
    assert!(!is_visual_media_decodable(&gif_bad_min_code_12), "GIF with min code size 12 must be rejected");

    // Broken dictionary codes (emits code 7 when next_code is 6)
    let gif_broken_dict = vec![
        b'G', b'I', b'F', b'8', b'9', b'a',
        0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00,
        0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF,
        0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
        0x02, 0x02, 0x7C, 0x01, 0x00, 0x3B,
    ];
    assert!(!is_visual_media_decodable(&gif_broken_dict), "GIF with broken dictionary code exceeding next_code must be rejected");

    let mut gif_truncated_lzw = valid_gif.clone();
    gif_truncated_lzw.truncate(valid_gif.len() - 4);
    assert!(!is_visual_media_decodable(&gif_truncated_lzw), "Truncated GIF without proper LZW EOI must be rejected");
}
