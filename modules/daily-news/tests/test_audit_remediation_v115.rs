//! Integration regression tests for v1.15.0 audit remediation:
//! - P0: Audit can be accepted even without a trusted issuer (reject unaccredited or non-matching issuer)
//! - P1: Visual asset has a valid checksum but the content is unusable (reject zero-dimension or blank assets)
//! - P1: API config returns the Reddit client secret (sanitize/mask secret in responses)
//! - P1: Consumed IDs and configuration may be lost after a restart (atomic writes and persistent recovery)
//! - P1: Semantic claim verification lacks sufficient reliability (detect semantic polarity conflicts)
//! - P2: MCP is not fully compatible with modern protocols (JSON-RPC 2.0 batch support and SSE route)

use daily_news::domain::{
    ClaimEvidence, ClaimGroundingStatus, VerificationRecord, VisualEvidence,
};
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::handlers::handle_request;
use daily_news::mcp::handle_jsonrpc_mcp_request;
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    is_audit_artifact_corroborated, is_valid_visual_asset_format,
    verify_claim_semantic_entailment, verify_visual_asset_record,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::url::compute_sha256_bytes;
use serde_json::json;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_v115_{tag}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn build_test_service(base_dir: &std::path::Path) -> DailyNewsService {
    let storage = Arc::new(FileStorageAdapter::new(base_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(base_dir));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();
    DailyNewsService::new(fetcher, storage, scorer).with_source_capture(capturer)
}

/// P0: Audit artifact must match an accredited issuer and corroborate the claim
#[test]
fn test_v115_audit_artifact_requires_accredited_issuer_match() {
    let claim = ClaimEvidence {
        claim_id: "claim_01".to_string(),
        statement: "Our benchmark achieved 4x throughput speedup on edge nodes".to_string(),
        evidence_refs: vec![],
        verification_status: "pending".to_string(),
        reviewer: None,
        verification_record: None,
    };

    let unaccredited_rec = VerificationRecord {
        record_id: "rec_unaccredited".to_string(),
        reviewer_id: "analyst_1".to_string(),
        organization: "Random Blog Lab".to_string(),
        verified_at: "2026-11-04T10:00:00Z".to_string(),
        method: "artifact_evaluation".to_string(),
        audit_trail_ref: Some("evidence/audit_unaccredited.txt".to_string()),
        secondary_source_ref: None,
    };
    let unaccredited_text = "Benchmark audit run by Random Blog Lab. Method artifact_evaluation verified reproducible throughput speedup across all test clusters with high confidence.";
    assert!(
        !is_audit_artifact_corroborated(unaccredited_text, &unaccredited_rec, &claim),
        "Audit artifact from unaccredited organization must be rejected"
    );

    let accredited_rec = VerificationRecord {
        record_id: "rec_csail".to_string(),
        reviewer_id: "prof_smith".to_string(),
        organization: "MIT CSAIL".to_string(),
        verified_at: "2026-11-04T10:00:00Z".to_string(),
        method: "artifact_evaluation".to_string(),
        audit_trail_ref: Some("evidence/audit_csail.txt".to_string()),
        secondary_source_ref: None,
    };
    let accredited_text = "MIT CSAIL independent audit confirmed the benchmark results. Method artifact_evaluation verified reproducible throughput speedup across test clusters.";
    assert!(
        is_audit_artifact_corroborated(accredited_text, &accredited_rec, &claim),
        "Audit artifact explicitly naming accredited organization and matching claim must pass"
    );

    let draft_text = "MIT CSAIL internal notes only. Method artifact_evaluation verified reproducible throughput speedup across test clusters.";
    assert!(
        !is_audit_artifact_corroborated(draft_text, &accredited_rec, &claim),
        "Audit artifact marked with draft or unverified tokens must fail closed"
    );
}

/// P1: Visual asset with valid checksum but unusable content must fail closed
#[test]
fn test_v115_visual_asset_unusable_content_fails_despite_valid_checksum() {
    // 1. All-zero payload with valid checksum
    let zero_payload = vec![0u8; 32];
    assert!(
        !is_valid_visual_asset_format(&zero_payload),
        "All-zero dummy filler must fail visual asset format validation"
    );

    // 2. PNG with zero dimensions in IHDR chunk
    let mut zero_dim_png = Vec::new();
    zero_dim_png.extend_from_slice(b"\x89PNG\r\n\x1a\n"); // 8 bytes PNG signature
    zero_dim_png.extend_from_slice(&13u32.to_be_bytes()); // Chunk length 13
    zero_dim_png.extend_from_slice(b"IHDR"); // Chunk type
    zero_dim_png.extend_from_slice(&0u32.to_be_bytes()); // Width = 0
    zero_dim_png.extend_from_slice(&0u32.to_be_bytes()); // Height = 0
    zero_dim_png.extend_from_slice(&[8, 2, 0, 0, 0]); // bit depth, color type, etc.

    let zero_dim_sha = compute_sha256_bytes(&zero_dim_png);
    assert!(
        !is_valid_visual_asset_format(&zero_dim_png),
        "PNG with zero width and zero height must fail format validation"
    );

    let visual = VisualEvidence {
        asset_ref: "chart_zero_dim".to_string(),
        kind: "diagram".to_string(),
        source_url: "https://example.org/chart.png".to_string(),
        source_snapshot_ref: Some("evidence/chart.png".to_string()),
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("VERIFIED".to_string()),
        license_evidence_ref: Some("https://creativecommons.org/licenses/by/4.0/".to_string()),
        verified_rights: Some(true),
        checksum_sha256: Some(zero_dim_sha),
    };
    let rec_res = verify_visual_asset_record(&visual, Some(&zero_dim_png), None);
    assert!(
        rec_res.is_err(),
        "verify_visual_asset_record must reject visual assets with unusable dimensions"
    );

    // 3. GIF with zero dimensions
    let mut zero_dim_gif = Vec::new();
    zero_dim_gif.extend_from_slice(b"GIF89a");
    zero_dim_gif.extend_from_slice(&0u16.to_le_bytes()); // Width = 0
    zero_dim_gif.extend_from_slice(&0u16.to_le_bytes()); // Height = 0
    assert!(
        !is_valid_visual_asset_format(&zero_dim_gif),
        "GIF with zero dimensions must fail format validation"
    );

    // 4. Valid PNG with positive dimensions
    let mut valid_png = Vec::new();
    valid_png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    valid_png.extend_from_slice(&13u32.to_be_bytes());
    valid_png.extend_from_slice(b"IHDR");
    valid_png.extend_from_slice(&800u32.to_be_bytes()); // Width = 800
    valid_png.extend_from_slice(&600u32.to_be_bytes()); // Height = 600
    valid_png.extend_from_slice(&[8, 2, 0, 0, 0]);

    let valid_sha = compute_sha256_bytes(&valid_png);
    assert!(
        is_valid_visual_asset_format(&valid_png),
        "Valid PNG with positive dimensions must pass format validation"
    );

    let valid_visual = VisualEvidence {
        asset_ref: "chart_valid".to_string(),
        kind: "diagram".to_string(),
        source_url: "https://example.org/valid_chart.png".to_string(),
        source_snapshot_ref: Some("evidence/valid_chart.png".to_string()),
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("VERIFIED".to_string()),
        license_evidence_ref: Some("https://creativecommons.org/licenses/by/4.0/".to_string()),
        verified_rights: Some(true),
        checksum_sha256: Some(valid_sha),
    };
    assert!(
        verify_visual_asset_record(&valid_visual, Some(&valid_png), None).is_ok(),
        "Valid visual asset must be verified successfully"
    );
}

/// P1: Config endpoint must sanitize Reddit client secret
#[test]
fn test_v115_api_config_sanitizes_reddit_secret() {
    let tmp = make_temp_dir("api_config");
    let service = build_test_service(&tmp);

    // Set an initial secret
    let update_req = json!({
        "path": "/config",
        "method": "POST",
        "body": {
            "reddit_client_id": "test_id_123",
            "reddit_client_secret": "super_secret_reddit_key_xyz"
        }
    });
    let post_resp = handle_request(update_req, &service, &tmp);
    assert_eq!(post_resp["status"], 200);

    let body_str = post_resp["body"].as_str().unwrap_or("");
    let parsed_post: serde_json::Value = serde_json::from_str(body_str).unwrap();
    assert_eq!(
        parsed_post["config"]["reddit_client_secret"],
        "********",
        "POST /config response must mask reddit_client_secret"
    );

    // GET /config must also mask secret
    let get_req = json!({
        "path": "/config",
        "method": "GET"
    });
    let get_resp = handle_request(get_req, &service, &tmp);
    assert_eq!(get_resp["status"], 200);
    let parsed_get: serde_json::Value =
        serde_json::from_str(get_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(
        parsed_get["reddit_client_secret"],
        "********",
        "GET /config response must mask reddit_client_secret"
    );

    // Echoing masked secret back in update must NOT overwrite real secret with asterisks
    let echo_masked_req = json!({
        "path": "/config",
        "method": "POST",
        "body": {
            "reddit_client_secret": "********",
            "freshness_window_hours": 48.0
        }
    });
    let echo_resp = handle_request(echo_masked_req, &service, &tmp);
    assert_eq!(echo_resp["status"], 200);

    // Verify underlying secret was preserved
    let underlying_cfg = service.get_config();
    assert_eq!(
        underlying_cfg.reddit_client_secret.as_deref(),
        Some("super_secret_reddit_key_xyz"),
        "Underlying secret must be preserved when masked placeholder is submitted"
    );
    assert_eq!(underlying_cfg.freshness_window_hours, 48.0);
}

/// P1: Consumed IDs and configuration must persist across storage restarts
#[test]
fn test_v115_consumed_and_config_persistence_across_restart() {
    let tmp = make_temp_dir("storage_restart");

    // 1. Initial storage instance: write consumed IDs and update config
    {
        let storage = FileStorageAdapter::new(&tmp);
        let _ = storage.mark_consumed(&["item_alpha".to_string(), "item_beta".to_string()]);
        assert!(storage.is_consumed("item_alpha"));

        let mut cfg = storage.load_config();
        cfg.reddit_client_id = Some("persisted_client_id".to_string());
        cfg.freshness_window_hours = 42.0;
        storage
            .save_config(&cfg)
            .expect("save_config should succeed");
    }

    // 2. Verify files exist in .state directory
    let state_dir = tmp.join(".state");
    assert!(state_dir.join("consumed.json").exists());
    assert!(state_dir.join("config.json").exists());

    // 3. New storage instance imitating application restart
    let recovered_storage = FileStorageAdapter::new(&tmp);
    assert!(
        recovered_storage.is_consumed("item_alpha"),
        "Consumed IDs must survive restart"
    );
    assert!(
        recovered_storage.is_consumed("item_beta"),
        "Consumed IDs must survive restart"
    );
    assert!(
        !recovered_storage.is_consumed("item_gamma"),
        "Unconsumed IDs must not be marked"
    );

    let recovered_cfg = recovered_storage.load_config();
    assert_eq!(
        recovered_cfg.reddit_client_id.as_deref(),
        Some("persisted_client_id"),
        "Config updates must survive restart"
    );
    assert_eq!(recovered_cfg.freshness_window_hours, 42.0);
}

/// P1: Semantic claim verification detects polarity conflicts
#[test]
fn test_v115_semantic_claim_polarity_conflict_rejected() {
    // Statement has "increased" while excerpt has "decreased"
    let opposing_statement = "System throughput increased by 25% under maximum cluster load";
    let excerpt = "The latest benchmark tests show that system throughput decreased by 25% under maximum cluster load";

    let opposing_status = verify_claim_semantic_entailment(opposing_statement, excerpt);
    assert_eq!(
        opposing_status,
        ClaimGroundingStatus::Contradicted,
        "Antonym/polarity mismatch must produce Contradicted status"
    );

    // Matching polarity
    let matching_statement = "System throughput decreased by 25% under maximum cluster load";
    let matching_status = verify_claim_semantic_entailment(matching_statement, excerpt);
    assert_eq!(
        matching_status,
        ClaimGroundingStatus::ClaimEntailed,
        "Matching polarity claim must produce ClaimEntailed status"
    );
}

/// P2: MCP protocol handles batch requests and SSE endpoint routes
#[test]
fn test_v115_mcp_batch_requests_and_sse_routes() {
    let tmp = make_temp_dir("mcp_batch");
    let service = build_test_service(&tmp);

    // 1. Empty batch must return HTTP 400 with -32600 Invalid Request
    let empty_batch = json!([]);
    let empty_resp = handle_jsonrpc_mcp_request(&service, &tmp, &empty_batch);
    assert_eq!(empty_resp["status"], 400);
    let parsed_err: serde_json::Value =
        serde_json::from_str(empty_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(parsed_err["error"]["code"], -32600);

    // 2. Batch with multiple requests
    let batch_req = json!([
        { "jsonrpc": "2.0", "id": 101, "method": "ping" },
        { "jsonrpc": "2.0", "id": 102, "method": "tools/list" }
    ]);
    let batch_resp = handle_jsonrpc_mcp_request(&service, &tmp, &batch_req);
    assert_eq!(batch_resp["status"], 200);
    let parsed_batch: serde_json::Value =
        serde_json::from_str(batch_resp["body"].as_str().unwrap()).unwrap();
    let batch_arr = parsed_batch.as_array().expect("must be array");
    assert_eq!(batch_arr.len(), 2);
    assert_eq!(batch_arr[0]["id"], 101);
    assert_eq!(batch_arr[1]["id"], 102);

    // 3. Batch with only notifications returns HTTP 204
    let notif_batch = json!([
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "method": "notifications/cancelled" }
    ]);
    let notif_resp = handle_jsonrpc_mcp_request(&service, &tmp, &notif_batch);
    assert_eq!(notif_resp["status"], 204);

    // 4. GET /mcp/sse and GET /sse return text/event-stream endpoint
    let sse_req = json!({
        "path": "/mcp/sse",
        "method": "GET"
    });
    let sse_resp = handle_request(sse_req, &service, &tmp);
    assert_eq!(sse_resp["status"], 200);
    assert_eq!(
        sse_resp["headers"]["content-type"],
        "text/event-stream"
    );
    assert!(sse_resp["body"].as_str().unwrap().contains("event: endpoint"));
    assert!(sse_resp["body"].as_str().unwrap().contains("data: /mcp"));

    let sse_root_req = json!({
        "path": "/sse",
        "method": "GET"
    });
    let sse_root_resp = handle_request(sse_root_req, &service, &tmp);
    assert_eq!(sse_root_resp["status"], 200);
    assert_eq!(
        sse_root_resp["headers"]["content-type"],
        "text/event-stream"
    );
}
