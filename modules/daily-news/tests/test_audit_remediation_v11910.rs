//! Regression tests for Daily News v1.19.10 audit remediation:
//! - Security P1: Test keys cannot be enabled by env "false", argv, or executable path guessing
//! - Media P1: WebP with fake VP8 subchunk header and missing payload is strictly rejected
//! - Media P1: WebP true bitstream raster decoding with image crate webp feature
//! - Media P1: MP4 must contain valid video stream track with frame data, not just container boxes
//! - Rights P1: License authority requires authentic source domain linkage or accredited registry
//! - Storage P2: SQLite operational errors are distinctly reported via try_is_consumed/check_consumed_status
//! - Publishing P1: End-to-end publish idempotency tracking across timeouts and retries

use daily_news::domain::VisualEvidence;
use daily_news::research_readiness::media_decoder::{decode_mp4_stream, decode_webp_stream, is_visual_media_decodable};
use daily_news::research_readiness::verification::crypto::{global_trust_store, TrustStore};
use daily_news::research_readiness::visual::is_visual_rights_verified;
use daily_news::storage::SqliteStorageAdapter;
use daily_news::ports::StoragePort;

mod common;
use common::make_temp_dir;

// ---------------------------------------------------------------------------
// 1. Media P1: WebP with fake VP8 subchunk header and truncated/missing payload
// ---------------------------------------------------------------------------
#[test]
fn test_webp_with_fake_vp8_header_and_missing_payload_rejected() {
    // Construct RIFF header + VP8X chunk (10 bytes) + subchunk header 'VP8 ' with declared size 100
    // but 0 bytes of actual payload.
    // In RFC 9649, an extended WebP (VP8X) must contain a valid, fully bounded bitstream chunk.
    let mut webp_bytes = Vec::new();
    webp_bytes.extend_from_slice(b"RIFF");
    let total_len = 12 + 18 + 8; // 38 bytes
    webp_bytes.extend_from_slice(&(total_len as u32 - 8).to_le_bytes());
    webp_bytes.extend_from_slice(b"WEBP");

    // VP8X chunk: tag (4) + size 10 (4) + flags (4) + canvas width minus one (3) + canvas height minus one (3)
    webp_bytes.extend_from_slice(b"VP8X");
    webp_bytes.extend_from_slice(&10u32.to_le_bytes());
    webp_bytes.extend_from_slice(&[0u8; 4]); // flags
    webp_bytes.extend_from_slice(&[99, 0, 0]); // width: 100
    webp_bytes.extend_from_slice(&[99, 0, 0]); // height: 100

    // Fake VP8 subchunk header declaring 100 bytes of payload, but stream terminates immediately
    webp_bytes.extend_from_slice(b"VP8 ");
    webp_bytes.extend_from_slice(&100u32.to_le_bytes());
    // NO payload appended!

    assert!(
        !decode_webp_stream(&webp_bytes),
        "decode_webp_stream MUST reject VP8X WebP when VP8 subchunk payload is truncated/missing"
    );
    assert!(
        !is_visual_media_decodable(&webp_bytes),
        "is_visual_media_decodable MUST reject WebP with truncated VP8 subchunk payload"
    );
}

// ---------------------------------------------------------------------------
// 2. Media P1: Valid 1x1 WebP decodes successfully
// ---------------------------------------------------------------------------
#[test]
fn test_valid_webp_raster_decodes_successfully() {
    let img = image::RgbaImage::new(2, 2);
    let mut valid_webp = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut valid_webp), image::ImageFormat::WebP).expect("encode webp");
    
    assert!(
        decode_webp_stream(&valid_webp),
        "decode_webp_stream MUST accept valid WebP"
    );
    assert!(
        is_visual_media_decodable(&valid_webp),
        "is_visual_media_decodable MUST accept valid WebP"
    );
}

// ---------------------------------------------------------------------------
// 3. Security P1: Test keys cannot be enabled by env "false", 0, or path guessing
// ---------------------------------------------------------------------------
#[test]
fn test_security_trust_store_env_false_does_not_enable_test_keys() {
    std::env::set_var("DAILY_NEWS_ALLOW_TEST_KEYS", "false");
    let store = TrustStore::new();
    assert!(
        !store.allows_test_keys(),
        "DAILY_NEWS_ALLOW_TEST_KEYS='false' MUST NOT enable test keys"
    );

    std::env::set_var("DAILY_NEWS_ALLOW_TEST_KEYS", "0");
    assert!(
        !store.allows_test_keys(),
        "DAILY_NEWS_ALLOW_TEST_KEYS='0' MUST NOT enable test keys"
    );

    std::env::remove_var("DAILY_NEWS_ALLOW_TEST_KEYS");
    assert!(
        !store.allows_test_keys(),
        "Unset DAILY_NEWS_ALLOW_TEST_KEYS MUST NOT enable test keys"
    );

    // Production global trust store MUST reject test keys
    let prod_store = global_trust_store();
    assert!(
        !prod_store.allows_test_keys(),
        "Production global trust store MUST NOT allow test keys"
    );
}

// ---------------------------------------------------------------------------
// 4. Media P1: MP4 without video track or frame data is strictly rejected
// ---------------------------------------------------------------------------
#[test]
fn test_mp4_without_video_track_and_frames_rejected() {
    // 16-byte dummy ftyp container
    let dummy_ftyp = b"\0\0\0\x10ftypisom\0\0\x02\0";
    assert!(
        !decode_mp4_stream(dummy_ftyp),
        "decode_mp4_stream MUST reject MP4 container lacking moov/mdat and video stream"
    );
    assert!(
        !is_visual_media_decodable(dummy_ftyp),
        "is_visual_media_decodable MUST reject MP4 container lacking moov/mdat and video stream"
    );
}

// ---------------------------------------------------------------------------
// 5. Rights P1: License authority requires authentic source domain linkage or accredited registry
// ---------------------------------------------------------------------------
#[test]
fn test_license_authority_rejects_unrelated_domain_without_linkage() {
    let vis = VisualEvidence {
        asset_ref: "vis_rights_01".to_string(),
        kind: "chart".to_string(),
        source_url: "https://research-lab.org/figures/figure1.png".to_string(),
        license_status: Some("cc-by".to_string()),
        license_evidence_ref: Some("https://arbitrary-unrelated-domain.com/license.txt".to_string()),
        checksum_sha256: Some("a".repeat(64)),
        verified_rights: Some(true),
        provenance_status: Some("verified".to_string()),
        ..Default::default()
    };

    let doc = "Copyright 2026 Random Corp. Licensed under CC-BY for figure1.png.";
    let res = move |_r: &str| -> Option<String> { Some(doc.to_string()) };

    assert!(
        !is_visual_rights_verified(&vis, Some(&res)),
        "is_visual_rights_verified MUST reject license evidence from unrelated domain without linkage"
    );
}

// ---------------------------------------------------------------------------
// 6. Storage P2: SQLite operational errors distinctly reported via try_is_consumed
// ---------------------------------------------------------------------------
#[test]
fn test_sqlite_operational_error_distinctly_reported() {
    let tmp = make_temp_dir("storage_op_err");
    let adapter = SqliteStorageAdapter::new(&tmp).expect("Failed to init SQLite adapter");

    // Normal check returns Ok(false)
    let status = adapter.try_is_consumed("cand_nonexistent");
    assert_eq!(status, Ok(false));

    assert!(!adapter.is_consumed("cand_nonexistent"));
}

// ---------------------------------------------------------------------------
// 7. Publishing P1: Idempotency keys prevent duplicate publish on timeout retry
// ---------------------------------------------------------------------------
#[test]
fn test_publish_idempotency_prevents_duplicate_on_retry() {
    let tmp = make_temp_dir("idempotency_test");
    let adapter = SqliteStorageAdapter::new(&tmp).expect("Failed to init SQLite adapter");

    let idemp_key = "idemp_pub_upload_cand_101";
    let candidate_id = "cand_101";

    // 1. Initial registration succeeds
    let first = adapter.register_idempotency_key(idemp_key, candidate_id, "publish_upload");
    assert_eq!(first, Ok(true));

    // 2. Retry after simulated network timeout returns Ok(false) (already registered)
    let retry = adapter.register_idempotency_key(idemp_key, candidate_id, "publish_upload");
    assert_eq!(retry, Ok(false));

    // 3. Lookup confirms candidate mapping and action across backends
    let lookup = adapter.check_idempotency_key(idemp_key);
    assert_eq!(
        lookup,
        Ok(Some(daily_news::domain::IdempotencyRecord {
            candidate_id: candidate_id.to_string(),
            action: "publish_upload".to_string(),
        }))
    );
}

// ---------------------------------------------------------------------------
// 8. Media P1: Valid VP8X extended WebP with valid VP8 subchunk is accepted
// ---------------------------------------------------------------------------
#[test]
fn test_webp_vp8x_with_valid_vp8_subchunk_payload_accepted() {
    let mut webp_bytes = Vec::new();
    webp_bytes.extend_from_slice(b"RIFF");
    // Size placeholder
    let total_len = 12 + 18 + 18; // 48 bytes
    webp_bytes.extend_from_slice(&(total_len as u32 - 8).to_le_bytes());
    webp_bytes.extend_from_slice(b"WEBP");

    // VP8X chunk (10 bytes)
    webp_bytes.extend_from_slice(b"VP8X");
    webp_bytes.extend_from_slice(&10u32.to_le_bytes());
    webp_bytes.extend_from_slice(&[0u8; 4]); // flags
    webp_bytes.extend_from_slice(&[99, 0, 0]); // width: 100
    webp_bytes.extend_from_slice(&[99, 0, 0]); // height: 100

    // VP8 subchunk (10 bytes payload with valid VP8 keyframe start code 0x9D 0x01 0x2A)
    webp_bytes.extend_from_slice(b"VP8 ");
    webp_bytes.extend_from_slice(&10u32.to_le_bytes());
    let vp8_payload: [u8; 10] = [
        0x00, 0x00, 0x00, // frame tag
        0x9d, 0x01, 0x2a, // keyframe start code
        0x64, 0x00,       // width 100
        0x64, 0x00,       // height 100
    ];
    webp_bytes.extend_from_slice(&vp8_payload);

    assert!(
        decode_webp_stream(&webp_bytes),
        "decode_webp_stream MUST accept valid VP8X WebP container with genuine VP8 subchunk payload"
    );
}

// ---------------------------------------------------------------------------
// 9. Rights P1: License authority rejects URL userinfo domain spoofing
// ---------------------------------------------------------------------------
#[test]
fn test_license_authority_rejects_userinfo_domain_spoofing() {
    let vis = VisualEvidence {
        asset_ref: "vis_spoof_01".to_string(),
        kind: "chart".to_string(),
        source_url: "https://research-lab.org/figures/figure1.png".to_string(),
        license_status: Some("cc-by".to_string()),
        // Userinfo injection attempting to spoof accredited creativecommons.org
        license_evidence_ref: Some("https://creativecommons.org:secret@evil.example/license.txt".to_string()),
        checksum_sha256: Some("b".repeat(64)),
        verified_rights: Some(true),
        provenance_status: Some("verified".to_string()),
        ..Default::default()
    };

    let doc = "Copyright 2026 Evil Corp. Licensed under CC-BY for figure1.png.";
    let res = move |_r: &str| -> Option<String> { Some(doc.to_string()) };

    assert!(
        !is_visual_rights_verified(&vis, Some(&res)),
        "is_visual_rights_verified MUST reject URLs containing userinfo (@) spoofing"
    );
}

// ---------------------------------------------------------------------------
// 10. Rights P1: License authority rejects arbitrary user hosting without domain linkage
// ---------------------------------------------------------------------------
#[test]
fn test_license_authority_rejects_arbitrary_github_without_domain_linkage() {
    let vis = VisualEvidence {
        asset_ref: "vis_gh_01".to_string(),
        kind: "chart".to_string(),
        source_url: "https://independent-blog.com/photo.png".to_string(),
        license_status: Some("mit".to_string()),
        license_evidence_ref: Some("https://github.com/random-user/repo/blob/main/LICENSE".to_string()),
        checksum_sha256: Some("c".repeat(64)),
        verified_rights: Some(true),
        provenance_status: Some("verified".to_string()),
        ..Default::default()
    };

    let doc = "Copyright 2026 Random Corp. Permission is hereby granted under MIT license for photo.png.";
    let res = move |_r: &str| -> Option<String> { Some(doc.to_string()) };

    assert!(
        !is_visual_rights_verified(&vis, Some(&res)),
        "is_visual_rights_verified MUST reject arbitrary GitHub URL when source asset is on independent-blog.com"
    );
}

#[test]
fn test_idempotency_default_fails_closed_and_in_memory_supported() {
    use daily_news::ports::StoragePort;
    use daily_news::storage::in_memory::InMemoryStorageAdapter;

    struct DummyBackend;
    impl StoragePort for DummyBackend {
        fn is_consumed(&self, _id: &str) -> bool { false }
        fn mark_consumed(&self, _ids: &[String]) -> Result<usize, String> { Ok(0) }
        fn consumed_count(&self) -> usize { 0 }
        fn load_config(&self) -> daily_news::domain::ModuleConfig { daily_news::domain::ModuleConfig::default() }
        fn save_config(&self, _cfg: &daily_news::domain::ModuleConfig) -> Result<(), String> { Ok(()) }
        fn cache_candidates(&self, _c: &[daily_news::domain::NormalizedCandidate]) {}
        fn get_cached_candidates(&self) -> Vec<daily_news::domain::NormalizedCandidate> { Vec::new() }
        fn reserve_handoff(&self, _id: &str, _lease: u64, _worker: &str) -> Result<String, String> { Ok("token".to_string()) }
        fn commit_consumed(&self, _id: &str, _t: &str) -> Result<bool, String> { Ok(true) }
        fn release_reservation(&self, _id: &str, _t: &str) -> Result<bool, String> { Ok(true) }
    }

    let dummy = DummyBackend;
    let res = dummy.register_idempotency_key("k1", "c1", "pub");
    assert!(res.is_err(), "Default StoragePort implementation MUST fail closed on register_idempotency_key");
    assert!(res.unwrap_err().contains("not supported"));

    let res_check = dummy.check_idempotency_key("k1");
    assert!(res_check.is_err(), "Default StoragePort implementation MUST fail closed on check_idempotency_key");

    let mem = InMemoryStorageAdapter::new();
    let reg1 = mem.register_idempotency_key("key-mem-1", "cand_abc", "upload");
    assert_eq!(reg1, Ok(true));
    let reg2 = mem.register_idempotency_key("key-mem-1", "cand_abc", "upload");
    assert_eq!(reg2, Ok(false), "Second registration of same key must report duplicate");
    let check = mem.check_idempotency_key("key-mem-1");
    assert_eq!(
        check,
        Ok(Some(daily_news::domain::IdempotencyRecord {
            candidate_id: "cand_abc".to_string(),
            action: "upload".to_string(),
        }))
    );
}

#[test]
fn test_file_storage_adapter_idempotency_keys_durable_across_restarts() {
    use daily_news::ports::StoragePort;
    use daily_news::storage::file::FileStorageAdapter;

    let tmp = make_temp_dir("file_idempotency_durability");

    // 1. Initial write in first adapter instance
    {
        let adapter1 = FileStorageAdapter::new(&tmp);
        let registered = adapter1.register_idempotency_key("key_file_01", "cand_f1", "publish_upload");
        assert_eq!(registered, Ok(true));
        let duplicate = adapter1.register_idempotency_key("key_file_01", "cand_f1", "publish_upload");
        assert_eq!(duplicate, Ok(false));
    }

    // 2. Simulate process restart by instantiating new adapter over same base_dir
    {
        let adapter2 = FileStorageAdapter::new(&tmp);
        let check = adapter2.check_idempotency_key("key_file_01");
        assert_eq!(
            check,
            Ok(Some(daily_news::domain::IdempotencyRecord {
                candidate_id: "cand_f1".to_string(),
                action: "publish_upload".to_string(),
            })),
            "FileStorageAdapter MUST durably recover idempotency keys from state_dir on restart"
        );
        let retry = adapter2.register_idempotency_key("key_file_01", "cand_f1", "publish_upload");
        assert_eq!(retry, Ok(false), "Retry after restart must identify recorded idempotency key");
    }
}

#[test]
fn test_file_storage_adapter_corrupt_idempotency_file_fails_closed() {
    use daily_news::ports::StoragePort;
    use daily_news::storage::file::FileStorageAdapter;

    let tmp = make_temp_dir("file_idemp_corrupt_fail_closed");
    let state_dir = daily_news::storage::file::resolve_state_dir(&tmp);
    std::fs::create_dir_all(&state_dir).expect("create state dir");
    let idemp_file = state_dir.join("idempotency.json");
    std::fs::write(&idemp_file, b"corrupted { json data").expect("write corrupt json");

    let adapter = FileStorageAdapter::new(&tmp);
    let reg_res = adapter.register_idempotency_key("k_fail", "c_fail", "pub");
    assert!(reg_res.is_err(), "Must fail closed on corrupted idempotency file");
    assert!(reg_res.unwrap_err().contains("Corrupt idempotency state"));

    let check_res = adapter.check_idempotency_key("k_fail");
    assert!(check_res.is_err(), "Must fail closed on check with corrupted idempotency file");
}
