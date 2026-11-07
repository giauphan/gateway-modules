//! Comprehensive regression tests for Daily News v1.19.11 audit remediation:
//! - P1 Media: ISO-BMFF recursive box validation rejects substring 'vide' in metadata,
//!   rejects tracks with 0 samples, and strictly accepts valid ISO-BMFF streams.
//! - P1 Publishing: End-to-end publish idempotency tracking wired through HTTP handlers
//!   and service, preventing duplicate publishing on retry.
//! - P1 Storage: State directory resolution strictly fails closed in production mode
//!   when the persistent directory is not writable, prohibiting silent /tmp fallback.
//! - P2 RPC: Strict JSON request body parsing rejects malformed syntax with 400 Bad Request.
//! - P2 Config: Support both JSON arrays and comma-separated string lists for config.

use daily_news::domain::{parse_string_list, NormalizedCandidate};
use daily_news::handlers::handle_request;
use daily_news::research_readiness::media_decoder::{decode_mp4_stream, is_visual_media_decodable};
use daily_news::rpc::try_parse_request_body;
use daily_news::service::DailyNewsService;
use daily_news::storage::file::try_resolve_state_dir;
use daily_news::storage::SqliteStorageAdapter;
use daily_news::StoragePort;
use serde_json::{json, Value};
use std::sync::Arc;

mod common;
use common::make_temp_dir;

// Helper to construct a big-endian ISO-BMFF box
fn make_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(8 + payload.len());
    let size = (8 + payload.len()) as u32;
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(box_type);
    data.extend_from_slice(payload);
    data
}

// Helper to construct a valid 28-byte VisualSampleEntry stsd box
fn make_valid_stsd_box(width: u16, height: u16) -> Vec<u8> {
    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&[0u8; 4]); // version & flags
    stsd_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1

    // First sample entry (VisualSampleEntry)
    let entry_len = 28u32;
    stsd_payload.extend_from_slice(&entry_len.to_be_bytes());
    stsd_payload.extend_from_slice(b"avc1");
    stsd_payload.extend_from_slice(&[0u8; 16]); // bytes 8..24
    stsd_payload.extend_from_slice(&width.to_be_bytes()); // bytes 24..26
    stsd_payload.extend_from_slice(&height.to_be_bytes()); // bytes 26..28

    make_box(b"stsd", &stsd_payload)
}

// ---------------------------------------------------------------------------
// 1. Media P1: Adversarial MP4 with 'vide' in non-video box rejected
// ---------------------------------------------------------------------------
#[test]
fn test_adversarial_mp4_vide_in_non_video_box_rejected() {
    let mut mp4 = Vec::new();
    // ftyp box
    mp4.extend(make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41"));
    // free box or udta containing deceptive 'vide' and 'vmhd' strings
    mp4.extend(make_box(b"free", b"deceptive content vide and vmhd fake stream"));
    // moov box with a trak containing an audio handler 'soun', NOT 'vide'
    let hdlr = make_box(b"hdlr", b"\0\0\0\0\0\0\0\0soun\0\0\0\0\0\0\0\0\0\0\0\0SoundHandler\0");
    let mdia = make_box(b"mdia", &hdlr);
    let trak = make_box(b"trak", &mdia);
    let moov = make_box(b"moov", &trak);
    mp4.extend(moov);
    // mdat with dummy bytes
    mp4.extend(make_box(b"mdat", &[1u8; 32]));

    assert!(
        !decode_mp4_stream(&mp4),
        "decode_mp4_stream MUST reject MP4 where 'vide' only exists inside non-video box"
    );
    assert!(
        !is_visual_media_decodable(&mp4),
        "is_visual_media_decodable MUST reject non-video MP4"
    );
}

// ---------------------------------------------------------------------------
// 2. Media P1: Video track with zero samples in stsz rejected
// ---------------------------------------------------------------------------
#[test]
fn test_adversarial_mp4_video_track_with_zero_samples_rejected() {
    let mut mp4 = Vec::new();
    mp4.extend(make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41"));

    // hdlr with handler_type = b"vide"
    let hdlr = make_box(b"hdlr", b"\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0VideoHandler\0");
    // vmhd
    let vmhd = make_box(b"vmhd", &[0u8; 12]);
    let stsd = make_valid_stsd_box(640, 360);

    // stsz with sample_count = 0
    let mut stsz_payload = vec![0u8; 4]; // version & flags
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // sample_count = 0
    let stsz = make_box(b"stsz", &stsz_payload);

    let mut stbl_payload = Vec::new();
    stbl_payload.extend(stsd);
    stbl_payload.extend(stsz);
    let stbl = make_box(b"stbl", &stbl_payload);

    let mut minf_payload = Vec::new();
    minf_payload.extend(vmhd);
    minf_payload.extend(stbl);
    let minf = make_box(b"minf", &minf_payload);

    let mut mdia_payload = Vec::new();
    mdia_payload.extend(hdlr);
    mdia_payload.extend(minf);
    let mdia = make_box(b"mdia", &mdia_payload);

    let trak = make_box(b"trak", &mdia);
    let moov = make_box(b"moov", &trak);
    mp4.extend(moov);
    mp4.extend(make_box(b"mdat", &[2u8; 32]));

    assert!(
        !decode_mp4_stream(&mp4),
        "decode_mp4_stream MUST reject MP4 with video track having 0 samples in stsz"
    );
}

// ---------------------------------------------------------------------------
// 3. Media P1: Valid synthetic ISO-BMFF video track accepted
// ---------------------------------------------------------------------------
#[test]
fn test_valid_isobmff_mp4_with_video_track_and_samples_accepted() {
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let hdlr = make_box(b"hdlr", b"\0\0\0\0\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0VideoHandler\0");
    let vmhd = make_box(b"vmhd", &[0u8; 12]);
    let stsd = make_valid_stsd_box(1280, 720);

    // stsz with sample_count = 2
    let mut stsz_payload = vec![0u8; 4];
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (table format)
    stsz_payload.extend_from_slice(&2u32.to_be_bytes()); // sample_count = 2
    stsz_payload.extend_from_slice(&16u32.to_be_bytes()); // sample 1
    stsz_payload.extend_from_slice(&16u32.to_be_bytes()); // sample 2
    let stsz = make_box(b"stsz", &stsz_payload);

    // Initial stco with dummy offset to calculate exact moov length
    let mut dummy_stco_payload = vec![0u8; 4]; // version & flags
    dummy_stco_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    dummy_stco_payload.extend_from_slice(&0u32.to_be_bytes()); // placeholder offset
    let dummy_stco = make_box(b"stco", &dummy_stco_payload);

    let mut dummy_stbl = Vec::new();
    dummy_stbl.extend(&stsd);
    dummy_stbl.extend(&stsz);
    dummy_stbl.extend(&dummy_stco);
    let stbl = make_box(b"stbl", &dummy_stbl);

    let mut dummy_minf = Vec::new();
    dummy_minf.extend(&vmhd);
    dummy_minf.extend(stbl);
    let minf = make_box(b"minf", &dummy_minf);

    let mut dummy_mdia = Vec::new();
    dummy_mdia.extend(&hdlr);
    dummy_mdia.extend(minf);
    let mdia = make_box(b"mdia", &dummy_mdia);

    let trak = make_box(b"trak", &mdia);
    let moov = make_box(b"moov", &trak);

    let mdat_start = ftyp.len() + moov.len();
    // In decode_mp4_stream, mdat_range is (c_start, c_end) which is the payload range!
    // The mdat box header is 8 bytes, so payload starts at mdat_start + 8
    let mdat_sample_offset = (mdat_start + 8) as u32;

    // Now construct real stco pointing right to the start of the mdat payload
    let mut real_stco_payload = vec![0u8; 4];
    real_stco_payload.extend_from_slice(&1u32.to_be_bytes());
    real_stco_payload.extend_from_slice(&mdat_sample_offset.to_be_bytes());
    let real_stco = make_box(b"stco", &real_stco_payload);

    let mut real_stbl = Vec::new();
    real_stbl.extend(stsd);
    real_stbl.extend(stsz);
    real_stbl.extend(real_stco);

    let mut real_minf = Vec::new();
    real_minf.extend(vmhd);
    real_minf.extend(make_box(b"stbl", &real_stbl));

    let mut real_mdia = Vec::new();
    real_mdia.extend(hdlr);
    real_mdia.extend(make_box(b"minf", &real_minf));

    let real_moov = make_box(b"moov", &make_box(b"trak", &make_box(b"mdia", &real_mdia)));

    let mut mp4 = Vec::new();
    mp4.extend(ftyp);
    mp4.extend(real_moov);
    mp4.extend(make_box(b"mdat", &[3u8; 64]));

    assert!(
        decode_mp4_stream(&mp4),
        "decode_mp4_stream MUST accept valid ISO-BMFF MP4 with valid video track and samples"
    );
    assert!(
        is_visual_media_decodable(&mp4),
        "is_visual_media_decodable MUST accept valid ISO-BMFF MP4"
    );
}

// ---------------------------------------------------------------------------
// 4. Publishing P1: End-to-end publish idempotency prevents duplicate on retry
// ---------------------------------------------------------------------------
#[test]
fn test_end_to_end_publish_idempotency_via_handler() {
    let tmp = make_temp_dir("idempotency_handler_test");
    let storage = Arc::new(SqliteStorageAdapter::new(&tmp).expect("create test sqlite storage"));
    let service = Arc::new(DailyNewsService::new(
        Arc::new(daily_news::fetcher::LiveFeedFetcher::new(None)),
        storage.clone(),
        daily_news::scoring::CandidateScorer::default(),
    ));

    let candidate_id = "cand-idem-test-11911";
    let cand = NormalizedCandidate {
        candidate_id: candidate_id.to_string(),
        title: "Idempotency End-to-End Test Article".to_string(),
        canonical_url: "https://example.com/idempotency-test-article".to_string(),
        published_at_utc: "2026-11-07T00:00:00Z".to_string(),
        ..Default::default()
    };
    storage.save_candidates(&[cand]).expect("save candidate");

    let token = storage
        .reserve_handoff(candidate_id, 3600, "production_worker")
        .expect("reserve candidate");

    let idem_key = "tx-publish-idempotency-v11911-replay-001";

    // First publish attempt: commit-consumed with candidate_id, token and idempotency_key
    let req_1 = json!({
        "path": "/api/v1/commit-consumed",
        "method": "POST",
        "body": {
            "candidate_id": candidate_id,
            "token": token,
            "idempotency_key": idem_key
        }
    });

    let resp_1 = handle_request(req_1.clone(), &service, &tmp);
    assert_eq!(resp_1["status"], 200, "First commit must succeed with 200 OK");
    let body_1: Value = serde_json::from_str(resp_1["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_1["status"], "PUBLISHED");
    assert_eq!(body_1["candidate_id"], candidate_id);

    // Replay attempt (simulate network timeout & retry with exact same idempotency_key)
    let resp_2 = handle_request(req_1, &service, &tmp);
    assert_eq!(resp_2["status"], 200, "Replay with same idempotency key must succeed idempotently (200 OK)");
    let body_2: Value = serde_json::from_str(resp_2["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_2["status"], "PUBLISHED");
    assert_eq!(body_2["candidate_id"], candidate_id);
    assert_eq!(body_2["idempotent"], true);

    // Query idempotency status via GET /api/v1/publish-idempotency?key=...
    let req_query = json!({
        "path": format!("/api/v1/publish-idempotency?key={idem_key}"),
        "method": "GET"
    });
    let resp_query = handle_request(req_query, &service, &tmp);
    assert_eq!(resp_query["status"], 200);
    let body_query: Value = serde_json::from_str(resp_query["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_query["status"], "RECORDED");
    assert_eq!(body_query["exists"], true);
    assert_eq!(body_query["record"]["candidate_id"], candidate_id);
    assert_eq!(body_query["record"]["action"], "commit_handoff");

    // Registering duplicate key via POST /api/v1/publish-idempotency must be rejected with 409 DUPLICATE
    let req_dup = json!({
        "path": "/api/v1/publish-idempotency",
        "method": "POST",
        "body": {
            "key": idem_key,
            "candidate_id": candidate_id
        }
    });
    let resp_dup = handle_request(req_dup, &service, &tmp);
    assert_eq!(resp_dup["status"], 409, "Duplicate registration must be rejected with 409 Conflict");
    let body_dup: Value = serde_json::from_str(resp_dup["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_dup["status"], "DUPLICATE");
    assert_eq!(body_dup["key"], idem_key);
}

// ---------------------------------------------------------------------------
// 5. Storage P1: Production mode strictly fails closed on non-writable state dir
// ---------------------------------------------------------------------------
#[test]
fn test_state_dir_fail_closed_in_production_mode() {
    let non_writable_path = std::path::Path::new("/proc/sys/fs/non_writable_test_state_dir_11911");

    // In production mode (no DAILY_NEWS_ALLOW_EPHEMERAL_STATE), it MUST fail closed
    std::env::remove_var("DAILY_NEWS_ALLOW_EPHEMERAL_STATE");
    let result = try_resolve_state_dir(non_writable_path);
    assert!(
        result.is_err(),
        "try_resolve_state_dir MUST return Err when state dir is non-writable in production mode"
    );

    let err_msg = result.err().unwrap().to_string();
    assert!(
        err_msg.contains("Ephemeral fallback to /tmp is prohibited in production"),
        "Error message must clearly state that ephemeral fallback to /tmp is prohibited in production"
    );
}

// ---------------------------------------------------------------------------
// 6. RPC P2: Strict JSON parsing rejects malformed syntax with 400 Bad Request
// ---------------------------------------------------------------------------
#[test]
fn test_strict_rpc_request_body_rejects_malformed_json() {
    let malformed_req = json!({
        "body": "{ unquoted_key: 123, "
    });
    let result = try_parse_request_body(&malformed_req, "");
    assert!(
        result.is_err(),
        "try_parse_request_body MUST reject malformed JSON syntax"
    );
    assert!(result.err().unwrap().contains("Invalid JSON request body"));

    let valid_req = json!({
        "body": "{\"key\": \"value\", \"count\": 42}"
    });
    let parsed = try_parse_request_body(&valid_req, "").expect("parse valid json");
    assert_eq!(parsed["key"], "value");
    assert_eq!(parsed["count"], 42);

    let empty_req = json!({});
    let empty = try_parse_request_body(&empty_req, "").expect("parse empty");
    assert_eq!(empty, json!({}));
}

// ---------------------------------------------------------------------------
// 7. Config P2: Array of strings and comma-separated string list parsing
// ---------------------------------------------------------------------------
#[test]
fn test_config_string_list_supports_arrays_and_csv() {
    let arr_value = json!(["crypto", "ai", "macro"]);
    let arr_parsed = parse_string_list(&arr_value, "test_field", false).expect("parse array");
    assert_eq!(arr_parsed, vec!["crypto", "ai", "macro"]);

    let csv_value = json!("tech, web3, defi");
    let csv_parsed = parse_string_list(&csv_value, "test_field", false).expect("parse csv");
    assert_eq!(csv_parsed, vec!["tech", "web3", "defi"]);

    let empty_value = json!("");
    let empty_parsed = parse_string_list(&empty_value, "test_field", false).expect("parse empty");
    assert!(empty_parsed.is_empty());
}
