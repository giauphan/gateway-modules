//! Verification and hardening tests for daily-news v1.12.0 audit remediation.
//!
//! Covers:
//! - P0: Native MCP Protocol Conformance (JSON-RPC 2.0 framing, camelCase isError, error codes).
//! - P0: Trusted Verification Records (fake audit hash and uncorroborated reviewers rejected).
//! - P0: Semantic Claim Grounding (claims with real excerpt but unsubstantiated assertions rejected).
//! - P0: Visual Provenance & Asset Verification (non-existent assets, checksum mismatches fail).
//! - P1: Transactional Persistence & Rollback (disk failure propagates error, RAM remains unmutated).
//! - P1: Semantic Research Package Validation (negation and hypothetical tokens filtered out).

use daily_news::domain::{
    compute_sha256, compute_sha256_bytes, evaluate_readiness_full,
    evaluate_readiness_with_snapshot_store, verify_claim_semantic_entailment,
    verify_visual_asset_record, ClaimEvidence, ClaimGroundingStatus, ContentOrigin, EvidenceRef,
    PrimarySourceEvidence, ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::enrichment::build_substantive_research_package;
use daily_news::handlers::handle_request;
use daily_news::mcp::{handle_mcp_tool_request, McpToolResponse};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::FileStorageAdapter;
use daily_news::{NormalizedCandidate, RawNewsItem, StoragePort};
use serde_json::{json, Value};
use std::fs;
use std::sync::Arc;

struct MockFeed;
impl daily_news::ports::FeedFetcherPort for MockFeed {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(vec![])
    }
}

fn sample_visual(asset_ref: &str, kind: &str, source_url: &str) -> VisualEvidence {
    VisualEvidence {
        asset_ref: asset_ref.to_string(),
        kind: kind.to_string(),
        source_url: source_url.to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(compute_sha256("valid visual asset bytes")),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }
}

// =========================================================================
// 1. Phase 1: Native MCP Protocol Conformance (JSON-RPC 2.0 & isError)
// =========================================================================

#[test]
fn test_mcp_jsonrpc_protocol_conformance() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_rpc_{}", uuid::Uuid::new_v4()));
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let service = DailyNewsService::new(Arc::new(MockFeed), storage, CandidateScorer::default());

    // 1. Initialize method returns standard MCP protocol version & server info
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ai-agent-client", "version": "1.0.0" }
        }
    });
    let init_resp = handle_mcp_tool_request(&service, &tmp, &init_req, None);
    assert_eq!(init_resp["status"], 200);
    let init_body: Value = serde_json::from_str(init_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(init_body["jsonrpc"], "2.0");
    assert_eq!(init_body["id"], 1);
    assert_eq!(init_body["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init_body["result"]["serverInfo"]["name"], "daily-news");

    // 2. tools/list returns declared tools inside JSON-RPC envelope
    let list_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });
    let list_resp = handle_mcp_tool_request(&service, &tmp, &list_req, None);
    assert_eq!(list_resp["status"], 200);
    let list_body: Value = serde_json::from_str(list_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(list_body["jsonrpc"], "2.0");
    assert_eq!(list_body["id"], 2);
    let tools = list_body["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 6);

    // 3. tools/call dispatches correctly with params.name and params.arguments
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "discover_candidates",
            "arguments": {
                "limit": 2
            }
        }
    });
    let call_resp = handle_mcp_tool_request(&service, &tmp, &call_req, None);
    assert_eq!(call_resp["status"], 200);
    let call_body: Value = serde_json::from_str(call_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(call_body["jsonrpc"], "2.0");
    assert_eq!(call_body["id"], 3);
    assert_eq!(call_body["result"]["isError"], false);
    assert!(call_body["result"]["content"].is_array());

    // 4. tools/call error response formats camelCase isError and error content
    let err_call = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "get_candidate",
            "arguments": {
                "candidate_id": "cand_missing_404"
            }
        }
    });
    let err_resp = handle_mcp_tool_request(&service, &tmp, &err_call, None);
    let err_body: Value = serde_json::from_str(err_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(err_body["jsonrpc"], "2.0");
    assert_eq!(err_body["id"], 4);
    assert_eq!(err_body["result"]["isError"], true);
    assert!(err_body["result"]["content"][0]["text"].as_str().unwrap().contains("not found"));

    // 5. tools/call missing name returns JSON-RPC error -32602 (Invalid Params)
    let missing_name = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {}
    });
    let mn_resp = handle_mcp_tool_request(&service, &tmp, &missing_name, None);
    let mn_body: Value = serde_json::from_str(mn_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(mn_body["error"]["code"], -32602);

    // 6. Unknown method returns JSON-RPC error -32601 (Method Not Found)
    let unknown_method = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "non_existent_method"
    });
    let um_resp = handle_mcp_tool_request(&service, &tmp, &unknown_method, None);
    let um_body: Value = serde_json::from_str(um_resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(um_body["error"]["code"], -32601);

    // 7. McpToolResponse serializes camelCase isError matching standard MCP specification
    let error_resp = McpToolResponse::error("Failure occurred");
    let json_error = serde_json::to_value(&error_resp).unwrap();
    assert_eq!(json_error["isError"], true);
    assert_eq!(json_error["is_error"], true);

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_route_dispatch_via_handlers() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_route_{}", uuid::Uuid::new_v4()));
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let service = DailyNewsService::new(Arc::new(MockFeed), storage, CandidateScorer::default());

    // Streamable HTTP POST /mcp endpoint
    let http_req = json!({
        "path": "/mcp",
        "method": "POST",
        "body": serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/list"
        })).unwrap()
    });

    let resp = handle_request(http_req, &service, &tmp);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 99);
    assert_eq!(body["result"]["tools"].as_array().unwrap().len(), 6);

    let _ = fs::remove_dir_all(tmp);
}

// =========================================================================
// 2. Phase 2: Trusted Verification Records & Real Audit Artifacts
// =========================================================================

#[test]
fn test_fake_audit_hash_and_unverified_record_fails_closed() {
    let text = "Validated whitepaper content on zero-copy serialization.";
    let sha = compute_sha256(text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/whitepaper".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Zero-copy Whitepaper".to_string()),
        published_at: None,
        excerpt: Some(text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    let fake_audit_sha = "f".repeat(64);
    let claim = ClaimEvidence {
        claim_id: "c_fake_audit".to_string(),
        statement: "Zero-copy serialization".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha.clone(),
            excerpt: "zero-copy serialization".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "vr_spoof_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "auditor_01".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(fake_audit_sha.clone()),
            secondary_source_ref: None,
        }),
    };

    let visuals = vec![sample_visual("vis_1", "chart", "https://example.com/chart.png")];

    // Case 1: Audit resolver cannot find fake_audit_sha -> FAILS CLOSED with NeedsVerification!
    let empty_audit_resolver = |_h: &str| -> Option<String> { None };
    let readiness_fake = evaluate_readiness_full(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        &visuals,
        ContentOrigin::Article,
        None,
        Some(&empty_audit_resolver),
        None,
    );
    assert_eq!(readiness_fake.status, ReadinessStatus::NeedsVerification);
    assert_ne!(readiness_fake.status, ReadinessStatus::VideoReady);

    // Case 2: Genuine audit artifact exists with matching SHA and accredited verdict -> VIDEO_READY!
    let audit_artifact_text = "Official Benchmarking Laboratory Audit Report: VERIFIED findings confirmed for Zero-copy serialization reproducibility pass.";
    let real_audit_sha = compute_sha256(audit_artifact_text);

    let mut verified_claim = claim;
    verified_claim.verification_record.as_mut().unwrap().audit_trail_ref = Some(real_audit_sha.clone());

    let real_audit_resolver = move |h: &str| -> Option<String> {
        if h == real_audit_sha {
            Some(audit_artifact_text.to_string())
        } else {
            None
        }
    };

    // Under unified verification gate, without asset resolver, readiness holds at NeedsVisualEvidence
    let readiness_no_asset = evaluate_readiness_full(
        Some(&source),
        Some(text),
        std::slice::from_ref(&verified_claim),
        &visuals,
        ContentOrigin::Article,
        None,
        Some(&real_audit_resolver),
        None,
    );
    assert_eq!(readiness_no_asset.status, ReadinessStatus::NeedsVisualEvidence);
    assert_ne!(readiness_no_asset.status, ReadinessStatus::VideoReady);

    // When valid asset resolver is supplied alongside audit resolver:
    let asset_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x10\x08\x06\0\0\0\x1f\xf3\xffa";
    let asset_sha = daily_news::url::compute_sha256_bytes(asset_bytes);
    let mut visuals_resolved = visuals.clone();
    visuals_resolved[0].checksum_sha256 = Some(asset_sha.clone());
    let valid_asset_res = move |h: &str| -> Option<Vec<u8>> {
        if h == asset_sha { Some(asset_bytes.to_vec()) } else { None }
    };

    let readiness_real = evaluate_readiness_full(
        Some(&source),
        Some(text),
        &[verified_claim],
        &visuals_resolved,
        ContentOrigin::Article,
        None,
        Some(&real_audit_resolver),
        Some(&valid_asset_res),
    );
    // Since Benchmarking Laboratory is used, it validates as MockVerified (not VideoReady)
    assert_eq!(readiness_real.status, ReadinessStatus::MockVerified);
}

// =========================================================================
// 3. Phase 3: Semantic Claim Grounding (Grounded Excerpts vs Contradicted / Unsubstantiated)
// =========================================================================

#[test]
fn test_semantic_claim_grounding_rejects_unsupported_percentage_claim() {
    let snapshot_text = "Our engineering team deployed a multi-tier caching layer across the infrastructure.";
    let sha = compute_sha256(snapshot_text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/cache".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Caching Architecture".to_string()),
        published_at: None,
        excerpt: Some(snapshot_text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    // False claim asserts 90% latency reduction, citing real cache excerpt
    let false_claim = ClaimEvidence {
        claim_id: "c_fake_num".to_string(),
        statement: "Latency reduced by 90% across edge clusters".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha.clone(),
            excerpt: "deployed a multi-tier caching layer across the infrastructure".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "SOURCE_SUPPORTED".to_string(),
        reviewer: None,
        verification_record: None,
    };

    let visuals = vec![sample_visual("vis_1", "chart", "https://example.com/chart.png")];

    // Must fail closed with NeedsResearch because 90% is unsubstantiated!
    let readiness = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(snapshot_text),
        &[false_claim],
        &visuals,
        ContentOrigin::Article,
        None,
    );

    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
    assert!(readiness.reasons[0].contains("lack validated evidence references"));
}

#[test]
fn test_semantic_claim_entailment_evaluator() {
    // 1. Metric assertion absent in excerpt yields ExcerptFound (not ClaimEntailed)
    let status_missing_num = verify_claim_semantic_entailment(
        "Latency dropped by 90%",
        "We deployed an updated redis cache cluster."
    );
    assert_eq!(status_missing_num, ClaimGroundingStatus::ExcerptFound);

    // 2. Metric assertion present in excerpt yields ClaimEntailed
    let status_valid = verify_claim_semantic_entailment(
        "Latency dropped by 90%",
        "We measured p99 latency across all edge nodes and observed a 90% reduction."
    );
    assert_eq!(status_valid, ClaimGroundingStatus::ClaimEntailed);

    // 3. Contradicted statement yields Contradicted
    let status_contradicted = verify_claim_semantic_entailment(
        "Observed empirical benchmark outcome",
        "The proposed design has not been benchmarked."
    );
    assert_eq!(status_contradicted, ClaimGroundingStatus::Contradicted);
}

// =========================================================================
// 4. Phase 4: Visual Provenance & Asset Verification
// =========================================================================

#[test]
fn test_visual_provenance_rejects_nonexistent_or_checksum_mismatch() {
    let text = "Article text on compiler performance.";
    let sha = compute_sha256(text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/compiler".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Compiler Speedup".to_string()),
        published_at: None,
        excerpt: Some(text.to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        captured_at: None,
        extraction_method: None,
    };

    let audit_text = "Verified official audit report from CertiK: audit findings independently confirmed for compiler performance benchmarking.";
    let audit_sha = compute_sha256(audit_text);
    let claim = ClaimEvidence {
        claim_id: "c_valid".to_string(),
        statement: "Compiler performance".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha.clone(),
            excerpt: "compiler performance".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "vr_comp_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "auditor_comp".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some(audit_sha.clone()),
            secondary_source_ref: None,
        }),
    };

    let audit_resolver = move |h: &str| -> Option<String> {
        if h == audit_sha {
            Some(audit_text.to_string())
        } else {
            None
        }
    };

    // Valid 68-byte decodable 1x1 RGBA PNG
    let real_image_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let real_image_sha = compute_sha256_bytes(real_image_bytes);

    let visual = VisualEvidence {
        asset_ref: "chart_01".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(real_image_sha.clone()),
        license_evidence_ref: Some("https://example.com/terms".to_string()),
        verified_rights: Some(true),
    };

    // Case 1: Asset resolver returns None (non-existent asset in storage) -> Fails closed with NeedsVisualEvidence!
    let empty_asset_resolver = |_sha: &str| -> Option<Vec<u8>> { None };
    let r_missing_asset = evaluate_readiness_full(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        std::slice::from_ref(&visual),
        ContentOrigin::Article,
        None,
        Some(&audit_resolver),
        Some(&empty_asset_resolver),
    );
    assert_eq!(r_missing_asset.status, ReadinessStatus::NeedsVisualEvidence);

    // Case 2: Asset resolver returns corrupted bytes (checksum mismatch) -> Fails closed!
    let corrupted_asset_resolver = |_sha: &str| -> Option<Vec<u8>> { Some(b"CORRUPTED BYTES".to_vec()) };
    let r_corrupted_asset = evaluate_readiness_full(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        std::slice::from_ref(&visual),
        ContentOrigin::Article,
        None,
        Some(&audit_resolver),
        Some(&corrupted_asset_resolver),
    );
    assert_eq!(r_corrupted_asset.status, ReadinessStatus::NeedsVisualEvidence);

    // Case 3: Asset bytes match checksum -> VIDEO_READY!
    let expected_img_sha = real_image_sha.clone();
    let valid_asset_resolver = move |sha: &str| -> Option<Vec<u8>> {
        if sha == expected_img_sha {
            Some(real_image_bytes.to_vec())
        } else {
            None
        }
    };
    let r_verified_asset = evaluate_readiness_full(
        Some(&source),
        Some(text),
        &[claim],
        std::slice::from_ref(&visual),
        ContentOrigin::Article,
        None,
        Some(&audit_resolver),
        Some(&valid_asset_resolver),
    );
    assert_eq!(r_verified_asset.status, ReadinessStatus::VideoReady);

    // Case 4: verify_visual_asset_record produces VerifiedAssetRecord
    let record_res = verify_visual_asset_record(&visual, Some(real_image_bytes), Some("Full CC-BY license agreement"));
    assert!(record_res.is_ok());
    let verified_record = record_res.unwrap();
    assert_eq!(verified_record.checksum_sha256, real_image_sha);
    assert!(verified_record.bytes_verified);
    assert!(verified_record.license_verified);
}

// =========================================================================
// 5. Phase 5: Transactional Persistence & Error Propagation
// =========================================================================

#[test]
fn test_transactional_persistence_leaves_ram_unmutated_on_disk_error() {
    let tmp = std::env::temp_dir().join(format!("test_transact_{}", uuid::Uuid::new_v4()));
    let storage = FileStorageAdapter::new(&tmp);

    // 1. Initial successful candidate save
    let initial_candidate = NormalizedCandidate {
        candidate_id: "cand_initial_01".to_string(),
        title: "Initial Candidate".to_string(),
        canonical_url: "https://example.com/initial".to_string(),
        ..Default::default()
    };
    let save_ok = storage.save_candidates(std::slice::from_ref(&initial_candidate));
    assert!(save_ok.is_ok());
    assert_eq!(storage.get_cached_candidates().len(), 1);

    // 2. Induce disk error by turning candidates.json into a non-writable directory
    let candidates_path = tmp.join(".state").join("candidates.json");
    let _ = fs::remove_file(&candidates_path);
    let _ = fs::create_dir_all(&candidates_path);

    // 3. Attempt to save new candidate
    let failing_candidate = NormalizedCandidate {
        candidate_id: "cand_failing_02".to_string(),
        title: "Failing Candidate".to_string(),
        canonical_url: "https://example.com/failing".to_string(),
        ..Default::default()
    };
    let save_err = storage.save_candidates(&[failing_candidate]);
    assert!(save_err.is_err(), "Must return Err on persistence failure");

    // 4. CRITICAL: Verify RAM cache was NOT mutated! Only initial candidate remains
    let cached_after_failure = storage.get_cached_candidates();
    assert_eq!(cached_after_failure.len(), 1);
    assert_eq!(cached_after_failure[0].candidate_id, "cand_initial_01");

    let _ = fs::remove_dir_all(&candidates_path);
    let _ = fs::remove_dir_all(&tmp);
}

// =========================================================================
// 6. Phase 6: Research Package Semantic Validation (Negation & Hypotheses)
// =========================================================================

#[test]
fn test_research_package_filters_negated_and_hypothetical_outcomes() {
    let unbenchmarked_text = "The team proposed an algorithm for network routing. The proposed design has not been benchmarked. Future work includes measuring throughput.";
    let sha = compute_sha256(unbenchmarked_text);

    let (claims, storyboard) = build_substantive_research_package(
        "cand_negated_test",
        &sha,
        "The team proposed an algorithm",
        "Network Routing",
        unbenchmarked_text,
        Some(".state/evidence/sample.txt"),
    );

    // "The proposed design has not been benchmarked" MUST NOT become an empirical outcome claim!
    assert!(
        !claims.iter().any(|c| c.claim_id.contains("claim_outcome")),
        "Negated benchmark sentence must not be extracted as an outcome claim"
    );

    // Root cause must not match if negated
    assert!(storyboard.root_cause.is_none());
}
